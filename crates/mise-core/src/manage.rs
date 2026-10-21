use crate::error::CoreError;
use std::path::PathBuf;

/// What `mise use --dry-run` would do: which tool specs get installed and
/// which config files change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsePreview {
    /// Tool specs that would be installed (e.g. `node@20.11.0`).
    pub install: Vec<String>,
    /// `mise.toml` files that would change.
    pub config_updates: Vec<PathBuf>,
    /// The raw dry-run output, for the terminal panel.
    pub raw: String,
}

/// Parse the plain-text output of `mise use --dry-run`.
///
/// Recognized line shapes (everything else is kept in `raw` but ignored
/// structurally; empty input is not an error):
///
/// ```text
/// mise node@20.11.0  ⇢ would install
/// mise would update /proj/mise.toml (add: node@20.11.0)
/// ```
pub fn parse_use_dry_run(text: &str) -> Result<UsePreview, CoreError> {
    let mut preview = UsePreview {
        install: Vec::new(),
        config_updates: Vec::new(),
        raw: text.to_string(),
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("mise would update ") {
            // `mise would update /proj/mise.toml (add: node@20.11.0)`
            let path = rest.split_once(" (").map_or(rest, |(p, _)| p).trim();
            if !path.is_empty() {
                preview.config_updates.push(PathBuf::from(path));
            }
            continue;
        }
        if line.contains("would install") {
            // `mise node@20.11.0  ⇢ would install`
            // (also tolerate `mise would install node@20.11.0`)
            if let Some((before, _)) = line.split_once(MISE_ARROW) {
                let spec = before.trim().strip_prefix("mise ").map(str::trim);
                if let Some(spec) = spec.filter(|s| !s.is_empty()) {
                    preview.install.push(spec.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("mise would install ") {
                let spec = rest.trim();
                if !spec.is_empty() {
                    preview.install.push(spec.to_string());
                }
            }
        }
    }
    Ok(preview)
}

/// The arrow character mise prints before `would install`.
const MISE_ARROW: char = '⇢';

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_would_install_line() {
        let text = "mise node@20.11.0  ⇢ would install";
        let p = parse_use_dry_run(text).unwrap();
        assert_eq!(p.install, vec!["node@20.11.0".to_string()]);
        assert!(p.config_updates.is_empty());
        assert_eq!(p.raw, text);
    }

    #[test]
    fn parses_would_update_line() {
        let text = "mise would update /proj/mise.toml (add: node@20.11.0)";
        let p = parse_use_dry_run(text).unwrap();
        assert!(p.install.is_empty());
        assert_eq!(p.config_updates, vec![PathBuf::from("/proj/mise.toml")]);
    }

    #[test]
    fn parses_both_shapes_across_lines() {
        let text = "mise node@20.11.0  ⇢ would install\n\
                    mise would update /proj/mise.toml (add: node@20.11.0)";
        let p = parse_use_dry_run(text).unwrap();
        assert_eq!(p.install, vec!["node@20.11.0".to_string()]);
        assert_eq!(p.config_updates, vec![PathBuf::from("/proj/mise.toml")]);
    }

    #[test]
    fn empty_input_is_empty_preview_not_error() {
        let p = parse_use_dry_run("").unwrap();
        assert!(p.install.is_empty());
        assert!(p.config_updates.is_empty());
        assert_eq!(p.raw, "");
    }

    #[test]
    fn unrelated_lines_are_ignored() {
        let text = "some unrelated banner\n\
                    mise python@3.12  ⇢ would install\n\
                    another stray line\n";
        let p = parse_use_dry_run(text).unwrap();
        assert_eq!(p.install, vec!["python@3.12".to_string()]);
        assert!(p.config_updates.is_empty());
    }
}
