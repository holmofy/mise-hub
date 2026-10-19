//! Internationalization scaffolding.
//!
//! The `rust_i18n::i18n!` macro itself is invoked at the crate root in
//! `main.rs`: its generated lookup machinery (`_rust_i18n_t!`, the backend, and
//! the `_rust_i18n_*` helpers) is emitted with `crate::`-relative paths, so it
//! must live at the crate root for `t!` to resolve. This module keeps the
//! runtime entry point (and, in Task 9, the user-facing locale handling).

/// Initialize the internationalization layer for the application.
///
/// Task 9 will drive this from user settings; for now the application always
/// starts in English.
pub fn init(locale: &str) {
    rust_i18n::set_locale(locale);
}
