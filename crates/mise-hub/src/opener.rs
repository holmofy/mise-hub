//! Cross-platform "open this URL in the browser" helper (M5).
//!
//! The settings page's update flow ends with the user downloading the new
//! release themselves, so all we need is to hand a URL to the platform
//! opener. URL validation is deliberately strict (http/https only, no
//! whitespace): the opener executes a system handler, and a `file://` or
//! custom-scheme URL must never reach it.

/// Only plain http(s) URLs without whitespace may be handed to the OS
/// opener. Everything else (empty string, `file:`, `javascript:`, paths,
/// embedded whitespace) is refused.
pub fn is_openable_url(url: &str) -> bool {
    let url = url.trim();
    let scheme_ok = url.starts_with("http://") || url.starts_with("https://");
    scheme_ok && !url.chars().any(char::is_whitespace)
}

/// The platform command that opens `url` in the user's browser, as a pure
/// `(program, args)` pair so tests can pin the argv without spawning:
/// macOS → `open`, Windows → `cmd /C start "" <url>` (the empty title
/// argument keeps `start` from swallowing a quoted URL as a window title),
/// everything else → `xdg-open`. The URL is passed verbatim as its own
/// argument — never through a shell string.
pub fn opener_command(url: &str) -> (String, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open".into(), vec![url.into()])
    } else if cfg!(target_os = "windows") {
        (
            "cmd".into(),
            vec!["/C".into(), "start".into(), String::new(), url.into()],
        )
    } else {
        ("xdg-open".into(), vec![url.into()])
    }
}

/// Open `url` with the platform browser opener. Refuses anything
/// [`is_openable_url`] rejects; the opener process is spawned and detached
/// (we never wait for the browser to exit).
pub fn open_url(url: &str) -> Result<(), String> {
    if !is_openable_url(url) {
        return Err(format!("refusing to open non-http(s) URL: {url}"));
    }
    let (program, args) = opener_command(url);
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_http_and_https_only() {
        assert!(is_openable_url("https://example.com/releases"));
        assert!(is_openable_url("http://example.com"));
        assert!(is_openable_url("  https://example.com  "), "trims input");
    }

    #[test]
    fn refuses_everything_that_is_not_plain_http() {
        assert!(!is_openable_url(""));
        assert!(!is_openable_url("   "));
        assert!(!is_openable_url("file:///etc/passwd"));
        assert!(!is_openable_url("javascript:alert(1)"));
        assert!(!is_openable_url("ftp://example.com"));
        assert!(!is_openable_url("/local/path"));
        assert!(
            !is_openable_url("https://example.com/a b"),
            "embedded whitespace is refused"
        );
    }

    #[test]
    fn opener_passes_the_url_verbatim_as_its_own_argument() {
        let url = "https://example.com/releases/latest";
        let (program, args) = opener_command(url);
        assert!(!program.is_empty());
        assert_eq!(args.last().map(String::as_str), Some(url));
        // The URL is exactly one argv entry — never split or shell-joined.
        assert_eq!(args.iter().filter(|a| a.as_str() == url).count(), 1);
    }

    #[test]
    fn windows_opener_uses_an_empty_title_argument() {
        if !cfg!(target_os = "windows") {
            return;
        }
        let (_, args) = opener_command("https://example.com");
        assert_eq!(args, vec!["/C", "start", "", "https://example.com"]);
    }

    #[test]
    fn open_url_refuses_invalid_urls_without_spawning() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("").is_err());
    }
}
