//! Theme selection built on top of the GPUI component theme registry.
//!
//! The registry is seeded with the built-in themes ("Default Light" and
//! "Default Dark") by `gpui_kit::init`. Selecting a theme applies it to the
//! global [`Theme`] immediately; an unknown name falls back to the default
//! light theme.

use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Window};

/// All available theme names, sorted the same way the registry presents them
/// (default themes first, light before dark, then by name).
pub fn theme_names(cx: &App) -> Vec<String> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// Apply the theme named `name`.
///
/// Looks the name up in the registry and, when found, installs that theme
/// configuration through [`Theme::update`], which refreshes every window.
/// Unknown names fall back to the default light theme.
pub fn apply_theme(name: &str, _window: &mut Window, cx: &mut App) {
    let config = ThemeRegistry::global(cx).themes().get(name).cloned();
    match config {
        Some(config) => {
            Theme::update(cx, |theme| theme.apply_config(&config));
        }
        None => {
            Theme::change(ThemeMode::Light, None, cx);
        }
    }
}
