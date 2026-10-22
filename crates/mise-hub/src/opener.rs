//! Cross-platform "open this URL in the browser" helper (M5).
//!
//! The settings page's update flow ends with the user downloading the new
//! release themselves, so all we need is to hand a URL to the platform
//! opener. URL validation is deliberately strict (http/https only, no
//! whitespace): the opener executes a system handler, and a `file://` or
//! custom-scheme URL must never reach it.
//!
//! No platform implementation goes through a shell: the URL is always one
//! argv element of a program that takes it verbatim. (Windows'
//! `cmd /C start "" <url>` was rejected for exactly that reason — cmd.exe
//! re-parses the command line and would truncate a URL at `&`.)

/// Only plain http(s) URLs without whitespace may be handed to the OS
/// opener. Everything else (empty string, `file:`, `javascript:`, paths,
/// embedded whitespace) is refused. The scheme check is case-insensitive
/// (`HTTPS://…` is a valid http(s) URL).
pub fn is_openable_url(url: &str) -> bool {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let scheme_ok = lower.starts_with("http://") || lower.starts_with("https://");
    scheme_ok && !url.chars().any(char::is_whitespace)
}

/// The platform command that opens `url` in the user's browser, as a pure
/// `(program, args)` pair so tests can pin every platform's argv without
/// spawning anything:
///
/// - macOS → `open <url>`
/// - Windows → `rundll32 url.dll,FileProtocolHandler <url>` (the handler
///   API; no shell, so `&` in the URL cannot truncate it)
/// - everything else → `xdg-open <url>`
///
/// The URL is passed verbatim as its own argument.
pub fn opener_command(url: &str) -> (String, Vec<String>) {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    opener_command_for(os, url)
}

/// The pure per-platform implementation behind [`opener_command`]. `os` is
/// one of `"macos"`, `"windows"`, `"linux"`.
pub fn opener_command_for(os: &str, url: &str) -> (String, Vec<String>) {
    match os {
        "macos" => ("open".into(), vec![url.into()]),
        "windows" => (
            "rundll32".into(),
            vec!["url.dll,FileProtocolHandler".into(), url.into()],
        ),
        _ => ("xdg-open".into(), vec![url.into()]),
    }
}

/// Open `url` with the platform browser opener. Refuses anything
/// [`is_openable_url`] rejects; the opener process is spawned and detached
/// (we never wait for the browser to exit).
pub fn open_url(url: &str) -> Result<(), String> {
    let url = url.trim();
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
        assert!(
            is_openable_url("HTTPS://example.com"),
            "the scheme check is case-insensitive"
        );
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

    /// All three platforms are pinned in one test — the plan requires the
    /// argv of every platform to be asserted, not just the current one.
    #[test]
    fn every_platform_passes_the_url_verbatim_as_one_argument() {
        let url = "https://example.com/releases/latest";

        let (program, args) = opener_command_for("macos", url);
        assert_eq!(program, "open");
        assert_eq!(args, vec![url.to_string()]);

        let (program, args) = opener_command_for("windows", url);
        assert_eq!(program, "rundll32");
        assert_eq!(
            args,
            vec!["url.dll,FileProtocolHandler".to_string(), url.to_string()]
        );

        let (program, args) = opener_command_for("linux", url);
        assert_eq!(program, "xdg-open");
        assert_eq!(args, vec![url.to_string()]);

        // The URL is exactly one argv entry everywhere — never split, never
        // shell-joined.
        for os in ["macos", "windows", "linux"] {
            let (_, args) = opener_command_for(os, url);
            assert_eq!(args.iter().filter(|a| a.as_str() == url).count(), 1);
        }
    }

    /// No platform may hand the URL to a shell: `cmd.exe` would re-parse
    /// it and truncate at `&`.
    #[test]
    fn no_platform_goes_through_a_shell() {
        for os in ["macos", "windows", "linux"] {
            let (program, args) = opener_command_for(os, "https://x/releases?a=1&b=2");
            assert_ne!(program, "cmd", "{os} must not use cmd.exe");
            assert_ne!(program, "sh", "{os} must not use a POSIX shell");
            assert_ne!(program, "powershell", "{os} must not use a shell");
            assert!(
                args.last().unwrap().contains('&'),
                "the URL must survive verbatim (no shell splitting): {os}"
            );
        }
    }

    #[test]
    fn the_current_platform_picks_its_own_command() {
        let (program, args) = opener_command("https://example.com");
        let expected = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "rundll32"
        } else {
            "xdg-open"
        };
        assert_eq!(program, expected);
        assert_eq!(args.last().map(String::as_str), Some("https://example.com"));
    }

    #[test]
    fn open_url_refuses_invalid_urls_without_spawning() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("").is_err());
    }
}
