//! Internationalization scaffolding.
//!
//! The `rust_i18n::i18n!` macro itself is invoked at the crate root in
//! `main.rs`: its generated lookup machinery (`_rust_i18n_t!`, the backend, and
//! the `_rust_i18n_*` helpers) is emitted with `crate::`-relative paths, so it
//! must live at the crate root for `t!` to resolve. This module keeps the
//! runtime entry point and the user-facing locale handling.

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

#[cfg(test)]
mod tests {
    use super::*;

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
