//! Update-check data layer (M5).
//!
//! mise-hub never downloads or installs itself; it fetches a small JSON
//! manifest from a user-configured URL and tells the user whether a newer
//! release exists. Network I/O ([`fetch_manifest`]) is separated from the
//! decision logic ([`check_update`], [`is_newer`]) so everything that matters
//! is testable offline. `fetch_manifest` blocks — it may only be called from
//! a background thread.

use crate::error::CoreError;
use serde::Deserialize;

/// The on-the-wire manifest format, documented in `README.md`:
/// `{"version": "0.2.0", "page_url": "https://…"}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UpdateManifest {
    /// The latest released version, optionally prefixed with `v`.
    pub version: String,
    /// Where the user should go to download it (e.g. the releases page).
    #[serde(default)]
    pub page_url: Option<String>,
}

/// The result of comparing a manifest against the running version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    /// The manifest's version is not newer than the running one.
    UpToDate,
    /// A newer release exists.
    UpdateAvailable {
        latest: String,
        page_url: Option<String>,
    },
}

/// Parse a manifest document. An empty (or whitespace-only) version is an
/// error, not an implicit "up to date" — a broken manifest must never tell
/// the user they are current.
pub fn parse_manifest(json: &str) -> Result<UpdateManifest, CoreError> {
    let manifest: UpdateManifest = serde_json::from_str(json)?;
    if manifest.version.trim().is_empty() {
        return Err(CoreError::Other(
            "update manifest has an empty version".into(),
        ));
    }
    Ok(manifest)
}

/// Compare two versions by their leading purely-numeric dot segments
/// (`0.2.0-rc.1` compares as `0.2`, i.e. a pre-release never outranks its
/// final release). Missing segments count as 0, so `0.2` is newer than
/// `0.1.9`. A leading `v` is stripped. Segment order is numeric, not
/// lexicographic (`0.10.0` > `0.9.0`).
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn segments(version: &str) -> Vec<u64> {
        version
            .trim()
            .trim_start_matches('v')
            .split('.')
            .take_while(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            .filter_map(|s| s.parse().ok())
            .collect()
    }
    let latest = segments(latest);
    let current = segments(current);
    // Pad the shorter side with zeros, then compare segment by segment:
    // `0.2` vs `0.2.0` pads to equality and is NOT newer.
    let len = latest.len().max(current.len());
    for i in 0..len {
        let l = latest.get(i).copied().unwrap_or(0);
        let c = current.get(i).copied().unwrap_or(0);
        if l != c {
            return l > c;
        }
    }
    false
}

/// Run a full update check. The closure performs the (potentially blocking)
/// fetch; this function only parses and compares.
pub fn check_update(
    fetcher: impl FnOnce() -> Result<String, CoreError>,
    current: &str,
) -> Result<UpdateCheck, CoreError> {
    let manifest = parse_manifest(&fetcher()?)?;
    if is_newer(&manifest.version, current) {
        Ok(UpdateCheck::UpdateAvailable {
            latest: manifest.version,
            page_url: manifest.page_url,
        })
    } else {
        Ok(UpdateCheck::UpToDate)
    }
}

/// Blocking HTTP GET of the manifest document. May only be called from a
/// background thread; transport errors, non-2xx statuses and oversized or
/// malformed bodies surface as [`CoreError`] with the underlying message.
pub fn fetch_manifest(url: &str, timeout: std::time::Duration) -> Result<String, CoreError> {
    ureq::get(url)
        .timeout(timeout)
        .call()
        .map_err(|e| CoreError::Other(e.to_string()))?
        .into_string()
        .map_err(|e| CoreError::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_compares_numeric_segments() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.0"), "numeric, not lexicographic");
        assert!(is_newer("v0.2.0", "0.1.9"), "leading v is stripped");
        assert!(is_newer("0.2", "0.1.9"), "missing segments count as 0");
        assert!(
            !is_newer("0.2.0-rc.1", "0.2.0"),
            "a pre-release never outranks its final release"
        );
        assert!(!is_newer("0.2.0", "0.2.0-rc.1"), "the final release wins");
    }

    #[test]
    fn parses_manifest_and_defaults_page_url() {
        let m = parse_manifest(r#"{"version": "0.2.0", "page_url": "https://x"}"#).unwrap();
        assert_eq!(m.version, "0.2.0");
        assert_eq!(m.page_url.as_deref(), Some("https://x"));

        let m = parse_manifest(r#"{"version": "0.2.0"}"#).unwrap();
        assert_eq!(m.page_url, None);
    }

    #[test]
    fn empty_version_is_an_error_not_up_to_date() {
        assert!(parse_manifest(r#"{"version": "  "}"#).is_err());
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_manifest("not json").is_err());
    }

    #[test]
    fn check_update_reports_available_and_up_to_date() {
        let got = check_update(|| Ok(r#"{"version": "0.2.0"}"#.into()), "0.1.0").unwrap();
        assert_eq!(
            got,
            UpdateCheck::UpdateAvailable {
                latest: "0.2.0".into(),
                page_url: None
            }
        );

        let got = check_update(|| Ok(r#"{"version": "0.1.0"}"#.into()), "0.1.0").unwrap();
        assert_eq!(got, UpdateCheck::UpToDate);
    }

    #[test]
    fn check_update_propagates_fetch_failure() {
        let got = check_update(|| Err(CoreError::Other("offline".into())), "0.1.0");
        assert!(got.is_err());
        assert!(got.unwrap_err().to_string().contains("offline"));
    }

    #[test]
    fn fetch_manifest_talks_http_and_surfaces_transport_errors() {
        // Minimal one-shot HTTP server on the loopback interface: serves a
        // fixed manifest, then the listener is dropped.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = r#"{"version": "9.9.9"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                use std::io::{Read, Write};
                // Read the request head to completion first; writing before
                // the client finished sending can reset the connection on
                // some platforms.
                let mut buf = [0u8; 1024];
                let mut head = Vec::new();
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
                sock.write_all(response.as_bytes()).ok();
            }
        });

        let got = fetch_manifest(
            &format!("http://{addr}/manifest.json"),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(got, body);

        // A closed port must be an error, not a hang or a panic.
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        assert!(
            fetch_manifest(
                &format!("http://{dead_addr}/manifest.json"),
                std::time::Duration::from_secs(5)
            )
            .is_err()
        );
    }
}
