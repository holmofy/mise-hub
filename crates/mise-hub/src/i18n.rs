//! Internationalization scaffolding.
//!
//! The `rust_i18n::i18n!` macro itself is invoked at the crate root in
//! `main.rs`: its generated lookup machinery (`_rust_i18n_t!`, the backend, and
//! the `_rust_i18n_*` helpers) is emitted with `crate::`-relative paths, so it
//! must live at the crate root for `t!` to resolve. This module keeps the
//! runtime entry point and the user-facing locale handling.

#[cfg(test)]
use std::collections::BTreeSet;

/// Initialize the internationalization layer for the application.
///
/// Called once at startup with the locale loaded from settings. Later changes
/// go through [`set_locale`].
pub fn init(locale: &str) {
    rust_i18n::set_locale(locale);
}

/// Switch the active locale at runtime.
///
/// Text rendered with `t!` is resolved at render time, so callers should
/// re-render (e.g. `cx.notify()`) after switching.
pub fn set_locale(locale: &str) {
    rust_i18n::set_locale(locale);
}

/// Map a raw system locale string (as returned by `sys_locale::get_locale`) to
/// one of the two supported locale codes.
///
/// Any locale whose language subtag starts with `zh` (case-insensitive) maps to
/// `"zh-CN"`; everything else, including `None` (no system locale detected),
/// maps to `"en"`.
pub fn locale_from_system(raw: Option<&str>) -> &'static str {
    match raw.and_then(|locale| locale.get(..2)) {
        Some(prefix) if prefix.eq_ignore_ascii_case("zh") => "zh-CN",
        _ => "en",
    }
}

/// Collect the dotted key paths of a locale file.
///
/// The parser is intentionally minimal: it only understands the shape
/// this project uses (two-space indentation, `key:` or `key: value`
/// lines, and `>` / `|` block scalars whose body must be skipped).
/// Anything it cannot read is ignored — the tests' job is to compare
/// the two files' key sets, not to validate YAML.
///
/// Lives at module level (not inside `mod tests`) so view modules can
/// assert that the keys THEY render exist in every locale.
#[cfg(test)]
pub(crate) fn locale_keys(yaml: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    // (indent, key path) of the enclosing sections, outermost first.
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut block_indent: Option<usize> = None;

    for line in yaml.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Inside a block scalar the body is free text, not keys.
        if let Some(block) = block_indent {
            if indent > block {
                continue;
            }
            block_indent = None;
        }
        if trimmed.starts_with('-') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            continue;
        }
        while let Some((level, _)) = stack.last() {
            if *level >= indent {
                stack.pop();
            } else {
                break;
            }
        }
        let path = match stack.last() {
            Some((_, parent)) => format!("{parent}.{key}"),
            None => key.to_string(),
        };
        keys.insert(path.clone());
        stack.push((indent, path));

        let value = value.trim();
        if value.starts_with('>') || value.starts_with('|') {
            block_indent = Some(indent);
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two locale files must expose exactly the same keys: a key that
    /// exists in only one of them renders as the raw key in that language
    /// (AGENTS.md rule 8).
    #[test]
    fn locale_files_have_identical_key_sets() {
        let en = locale_keys(include_str!("../locales/en.yml"));
        let zh = locale_keys(include_str!("../locales/zh-CN.yml"));

        let missing_in_zh: Vec<&String> = en.difference(&zh).collect();
        let missing_in_en: Vec<&String> = zh.difference(&en).collect();
        assert!(
            missing_in_zh.is_empty(),
            "keys present in en.yml but missing in zh-CN.yml: {missing_in_zh:?}"
        );
        assert!(
            missing_in_en.is_empty(),
            "keys present in zh-CN.yml but missing in en.yml: {missing_in_en:?}"
        );
        assert!(!en.is_empty(), "the parser found no keys at all");
    }

    /// The parser must actually see the nested structure — a flat file
    /// would make the parity test vacuous.
    #[test]
    fn locale_keys_include_nested_paths() {
        let en = locale_keys(include_str!("../locales/en.yml"));
        assert!(en.contains("nav.discover"));
        assert!(en.contains("settings.probe.registry"));
        // Block scalars must not leak their body into the key set.
        assert!(!en.iter().any(|k| k.contains("Uninstall")));
    }

    #[test]
    fn locale_from_system_maps_zh_locales() {
        assert_eq!(locale_from_system(Some("zh-CN")), "zh-CN");
        assert_eq!(locale_from_system(Some("zh_Hans_CN")), "zh-CN");
    }

    #[test]
    fn locale_from_system_falls_back_to_english() {
        assert_eq!(locale_from_system(Some("en-US")), "en");
        assert_eq!(locale_from_system(None), "en");
    }
}
