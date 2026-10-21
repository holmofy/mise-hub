//! Environment-variable data layer: parsing of `mise env --json-extended`
//! and the plain-text `mise set` listing, plus PATH segmentation helpers.
//!
//! Note: mise DOES redact by default — a variable declared with
//! `redact = true` is listed by `mise set` as the literal `[redacted]` (the
//! app passes `--no-redact` to get raw values; see `Mise::env_declarations`).
//! The literal placeholder is still treated defensively everywhere: it is
//! never shown as a real value, never diffed as one, and never prefilled
//! into an edit dialog (see [`is_redacted`]).

use crate::error::CoreError;
use std::path::PathBuf;

use serde::Deserialize;

/// One row of `mise env --json-extended`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
    /// Non-empty source = declared in a config file.
    pub source: Option<PathBuf>,
    /// Non-empty tool = injected by an installed tool (e.g. `JAVA_HOME`).
    pub tool: Option<String>,
}

/// One row of the plain-text `mise set` listing.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvDeclaration {
    pub name: String,
    pub value: String,
    pub source: PathBuf,
    /// True when the column layout cannot be recovered with certainty: the
    /// line contained a multi-space run inside the value region (whitespace
    /// was collapsed while joining) or more than one token after the name
    /// looks like a path (the value may itself contain a path). The UI
    /// should call `Mise::env_value` for an exact read of such rows.
    pub ambiguous: bool,
}

/// One entry of `mise env --json-extended` for a single variable. Fields the
/// CLI omits (or leaves as `""`) decode to `None`: an empty `source` means the
/// value is not declared in a config file, an empty/missing `tool` means it is
/// not injected by an installed tool.
#[derive(Deserialize)]
struct RawEnvVar {
    #[serde(default)]
    source: String,
    #[serde(default)]
    tool: String,
    #[serde(default)]
    value: String,
}

/// Parses the JSON object of `mise env --json-extended` into [`EnvVar`] rows,
/// sorted by name. Non-object input (e.g. a bare array) is an error.
pub fn parse_env_extended(json: &str) -> Result<Vec<EnvVar>, CoreError> {
    let raw: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(json).map_err(CoreError::Json)?;
    let mut vars: Vec<EnvVar> = raw
        .into_iter()
        .map(|(name, v)| {
            let raw: RawEnvVar = serde_json::from_value(v).map_err(CoreError::Json)?;
            Ok(EnvVar {
                name,
                value: raw.value,
                source: non_empty_path(raw.source),
                tool: non_empty_string(raw.tool),
            })
        })
        .collect::<Result<_, CoreError>>()?;
    vars.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(vars)
}

fn non_empty_path(s: String) -> Option<PathBuf> {
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

fn non_empty_string(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// True when a value is the literal `[redacted]` placeholder `mise set`
/// prints for variables declared with `redact = true` (mise redacts by
/// default; the app requests `--no-redact`, so a surviving placeholder in
/// parsed data is a misread row or a stale listing — either way it must be
/// treated as "requires an exact read", never shown or written back).
pub fn is_redacted(value: &str) -> bool {
    value.trim() == "[redacted]"
}

/// True when a token looks like a config-file path: it starts with `/`, `~`,
/// or a Windows drive prefix (`X:\`). Those are the shapes `mise set` uses
/// for its source column.
fn looks_like_path(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with('~')
        || (token.len() >= 3
            && token.as_bytes()[1] == b':'
            && token.as_bytes()[2] == b'\\'
            && token.as_bytes()[0].is_ascii_alphabetic())
}

/// A whitespace-delimited token together with its byte range in the line.
struct Token<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

fn tokenize_with_offsets(line: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                tokens.push(Token {
                    start: s,
                    end: idx,
                    text: &line[s..idx],
                });
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(s) = start {
        tokens.push(Token {
            start: s,
            end: line.len(),
            text: &line[s..],
        });
    }
    tokens
}

/// Parses the plain-text `mise set` listing.
///
/// Layout rule (documented mise behavior, verified on 2026.10.15): each line
/// is `NAME VALUE… SOURCE`. The first whitespace-delimited token is the name;
/// the source is the RIGHTMOST token after the name that starts with `/`,
/// `~`, or a Windows drive prefix (`X:\`) — everything before it is the value
/// (odd look of `mise set P` when the value itself is path-shaped). Source
/// paths may contain spaces (macOS `/…/my dir/mise.toml`, Windows
/// `C:\Users\John Smith\…\mise.toml`), so every remaining token from the
/// source onward is joined back together. Empty values are preserved; a line
/// with a name but no path token anywhere is skipped.
///
/// Because whitespace inside the value is collapsed and path-shaped values can
/// shift where the source column begins, [`EnvDeclaration::ambiguous`] marks
/// rows whose layout cannot be recovered with certainty;
/// `Mise::env_value` (`mise set <NAME>`) is the documented fallback for an
/// exact read.
pub fn parse_set_list(text: &str) -> Result<Vec<EnvDeclaration>, CoreError> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let tokens = tokenize_with_offsets(line);
        if tokens.len() < 2 {
            continue;
        }
        let name = tokens[0].text;
        // Rightmost path-shaped token at index >= 1 is the source column.
        let Some(i) = tokens[1..]
            .iter()
            .rposition(|t| looks_like_path(t.text))
            .map(|p| p + 1)
        else {
            // No path token anywhere => no recoverable source column; skip.
            continue;
        };
        let value = tokens[1..i]
            .iter()
            .map(|t| t.text)
            .collect::<Vec<_>>()
            .join(" ");
        let source = tokens[i..]
            .iter()
            .map(|t| t.text)
            .collect::<Vec<_>>()
            .join(" ");
        // Ambiguous when: a multi-space run collapses inside the value region
        // (strictly between value tokens), or the value itself contains
        // another path-shaped token.
        let collapsed = tokens[1..i].windows(2).any(|pair| {
            line.get(pair[0].end..pair[1].start)
                .is_some_and(|gap| gap.chars().all(char::is_whitespace) && gap.chars().count() >= 2)
        });
        let multiple_paths = tokens[1..]
            .iter()
            .filter(|t| looks_like_path(t.text))
            .count()
            > 1;
        let redacted = is_redacted(&value);
        rows.push(EnvDeclaration {
            name: name.to_string(),
            value,
            source: PathBuf::from(source),
            ambiguous: collapsed || multiple_paths || redacted,
        });
    }
    Ok(rows)
}

/// Splits a PATH-like value on the platform separator, dropping empty
/// segments (consecutive/leading/trailing separators are common in real
/// values).
pub fn path_segments(value: &str) -> Vec<String> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    value
        .split(sep)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// A variable is a plain config declaration when it has a (non-empty) source
/// and is not injected by a tool. Tool-injected or system variables are not
/// editable declarations.
pub fn is_declared(v: &EnvVar) -> bool {
    v.source.is_some() && v.tool.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV_EXTENDED: &str = r#"{
        "FOO":       { "source": "/proj/mise.toml", "value": "bar" },
        "A":         { "source": "/proj/mise.toml", "value": "1" },
        "JAVA_HOME": { "source": "", "tool": "java", "value": "/opt/jdk" },
        "PATH":      { "value": "a:b:c" },
        "ZED":       { "source": "~/.config/mise/config.toml", "value": "x", "tool": "" }
    }"#;

    // ---- parse_env_extended ----

    #[test]
    fn env_extended_parses_config_source() {
        let vars = parse_env_extended(ENV_EXTENDED).unwrap();
        let foo = vars.iter().find(|v| v.name == "FOO").unwrap();
        assert_eq!(foo.value, "bar");
        assert_eq!(foo.source, Some(PathBuf::from("/proj/mise.toml")));
        assert_eq!(foo.tool, None);
    }

    #[test]
    fn env_extended_parses_tool_injected() {
        let vars = parse_env_extended(ENV_EXTENDED).unwrap();
        let java = vars.iter().find(|v| v.name == "JAVA_HOME").unwrap();
        assert_eq!(java.tool, Some("java".into()));
        // Empty source means "not from a config file".
        assert_eq!(java.source, None);
    }

    #[test]
    fn env_extended_parses_system_var() {
        let vars = parse_env_extended(ENV_EXTENDED).unwrap();
        let path = vars.iter().find(|v| v.name == "PATH").unwrap();
        assert_eq!(path.source, None);
        assert_eq!(path.tool, None);
        assert_eq!(path.value, "a:b:c");
    }

    #[test]
    fn env_extended_missing_tool_and_source_fields() {
        let vars = parse_env_extended(r#"{ "X": { "value": "v" } }"#).unwrap();
        assert_eq!(vars[0].tool, None);
        assert_eq!(vars[0].source, None);
    }

    #[test]
    fn env_extended_sorted_by_name() {
        let vars = parse_env_extended(ENV_EXTENDED).unwrap();
        let names: Vec<&str> = vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["A", "FOO", "JAVA_HOME", "PATH", "ZED"]);
    }

    #[test]
    fn env_extended_invalid_json_is_error() {
        assert!(parse_env_extended("not json").is_err());
        // A bare array is not the documented shape either.
        assert!(parse_env_extended("[]").is_err());
    }

    #[test]
    fn env_extended_empty_object_is_empty_list() {
        assert!(parse_env_extended("{}").unwrap().is_empty());
    }

    // ---- parse_set_list ----

    /// Values may contain spaces; the first token is the name, the last
    /// path-shaped token is the source, everything between is the value.
    const SET_TABLE: &str = "\
A         1            /proj/mise.toml
B         x y z        /proj/mise.toml
C         ~/x          ~/.config/mise/config.toml
D         win          C:\\Users\\x\\mise.toml
NAME_ONLY justvalue
";

    #[test]
    fn set_list_value_with_spaces() {
        let rows = parse_set_list(SET_TABLE).unwrap();
        let b = rows.iter().find(|r| r.name == "B").unwrap();
        assert_eq!(b.value, "x y z");
        assert_eq!(b.source, PathBuf::from("/proj/mise.toml"));
    }

    #[test]
    fn set_list_parses_home_and_windows_sources() {
        let rows = parse_set_list(SET_TABLE).unwrap();
        let c = rows.iter().find(|r| r.name == "C").unwrap();
        assert_eq!(c.source, PathBuf::from("~/.config/mise/config.toml"));
        let d = rows.iter().find(|r| r.name == "D").unwrap();
        assert_eq!(d.source, PathBuf::from("C:\\Users\\x\\mise.toml"));
    }

    #[test]
    fn set_list_plain_path_values_are_value() {
        // A source must start with `/`, `~`, or a drive prefix; `x` does not,
        // so for `NAME_ONLY justvalue` there is no valid source column and
        // the single-value line is skipped (fewer than 3 usable tokens).
        let rows = parse_set_list("NAME_ONLY justvalue\n").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn set_list_skips_too_few_tokens() {
        let rows = parse_set_list(SET_TABLE).unwrap();
        assert_eq!(rows.iter().filter(|r| r.name == "NAME_ONLY").count(), 0);
    }

    #[test]
    fn set_list_spaced_source_path() {
        // Source directory contains a space: the whole tail after the
        // rightmost path token is joined back into the source.
        let rows = parse_set_list("SPACED  v1  /a b/mise.toml\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "SPACED");
        assert_eq!(rows[0].value, "v1");
        assert_eq!(rows[0].source, PathBuf::from("/a b/mise.toml"));
        assert!(!rows[0].ambiguous);
    }

    #[test]
    fn set_list_empty_value_is_kept() {
        // `mise set EMPTY` (no value) prints a 2-token row; it must not be
        // skipped.
        let rows = parse_set_list("EMPTY  /proj/mise.toml\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "EMPTY");
        assert_eq!(rows[0].value, "");
        assert_eq!(rows[0].source, PathBuf::from("/proj/mise.toml"));
        assert!(!rows[0].ambiguous);
    }

    #[test]
    fn set_list_path_shaped_value() {
        // The value itself is path-shaped; the RIGHTMOST path token is the
        // source, everything between name and source is the value.
        let rows = parse_set_list("P  /usr/local/bin  /a b/mise.toml\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "P");
        assert_eq!(rows[0].value, "/usr/local/bin");
        assert_eq!(rows[0].source, PathBuf::from("/a b/mise.toml"));
        // The value may itself contain a path => ambiguous.
        assert!(rows[0].ambiguous);
    }

    #[test]
    fn set_list_collapsed_whitespace_is_ambiguous() {
        // A double space inside the value was collapsed while joining.
        let rows = parse_set_list("GAPPY  a  b  /proj/mise.toml\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "a b");
        assert!(rows[0].ambiguous);
    }

    #[test]
    fn set_list_redacted_row_defensively_ambiguous() {
        // mise redacts `redact = true` declarations as `[redacted]` by
        // default; such a row must be marked ambiguous so the UI never
        // shows / prefills / diffs the literal placeholder as a real value.
        let rows = parse_set_list("SECRET  [redacted]  /proj/mise.toml\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "SECRET");
        assert_eq!(rows[0].value, "[redacted]");
        assert!(rows[0].ambiguous);
        assert!(is_redacted(&rows[0].value));
        // SURROUNDING whitespace does not stop the placeholder from
        // counting as redacted.
        assert!(is_redacted(" [redacted] "));
        assert!(!is_redacted("[redacted-value]"));
        assert!(!is_redacted("hunter2"));
    }

    #[test]
    fn set_list_row_without_path_token_is_skipped() {
        let rows = parse_set_list("NO_PATH  just value\n").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn set_list_empty_input_is_empty_list() {
        assert!(parse_set_list("").unwrap().is_empty());
        assert!(parse_set_list("\n\n").unwrap().is_empty());
    }

    #[test]
    fn set_list_first_name_token_verbatim() {
        let rows = parse_set_list("MY_VAR  hello  /p/m.toml\n").unwrap();
        assert_eq!(rows[0].name, "MY_VAR");
        assert_eq!(rows[0].value, "hello");
    }

    // ---- path_segments ----

    #[test]
    fn path_segments_splits_on_platform_separator() {
        // Built to match the platform so the test holds on Unix and Windows.
        let joined = if cfg!(windows) { "a;b;c" } else { "a:b:c" };
        assert_eq!(path_segments(joined), vec!["a", "b", "c"]);
    }

    #[test]
    fn path_segments_unix_colon() {
        // Unix-only check: colon is the separator on non-Windows targets.
        if !cfg!(windows) {
            assert_eq!(path_segments("a:b:c"), vec!["a", "b", "c"]);
        }
    }

    #[test]
    fn path_segments_drops_empty_segments() {
        if !cfg!(windows) {
            assert_eq!(path_segments(":a::b:"), vec!["a", "b"]);
        }
    }

    #[test]
    fn path_segments_single_segment() {
        assert_eq!(path_segments("only"), vec!["only"]);
    }

    #[test]
    fn path_segments_empty_value() {
        assert!(path_segments("").is_empty());
        if !cfg!(windows) {
            assert!(path_segments("::").is_empty());
        }
    }

    // ---- is_declared truth table ----

    fn env_var(source: Option<&str>, tool: Option<&str>) -> EnvVar {
        EnvVar {
            name: "X".into(),
            value: "v".into(),
            source: source.map(PathBuf::from),
            tool: tool.map(String::from),
        }
    }

    #[test]
    fn is_declared_truth_table() {
        // Declared in config, not tool-injected => true.
        assert!(is_declared(&env_var(Some("/proj/mise.toml"), None)));
        // Tool-injected, empty source => false.
        assert!(!is_declared(&env_var(Some(""), Some("java"))));
        // Neither => false (system/unknown).
        assert!(!is_declared(&env_var(None, None)));
        // Both source and tool set => tool wins, not a plain declaration.
        assert!(!is_declared(&env_var(
            Some("/proj/mise.toml"),
            Some("java")
        )));
    }
}
