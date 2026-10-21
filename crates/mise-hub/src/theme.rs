//! Theme selection built on top of the GPUI component theme registry.
//!
//! The registry is seeded with the built-in themes ("Default Light" and
//! "Default Dark") by `gpui_kit::init`. Selecting a theme applies it to the
//! global [`Theme`] immediately; an unknown name falls back to the default
//! light theme.

use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Window};

/// Candidate monospace families, cross platform (no hardcoded macOS
/// assumption): macOS → Menlo, Windows → Consolas, Linux → DejaVu Sans
/// Mono, with a generic `monospace` fallback last.
#[allow(dead_code)] // the chain is pinned by the font-chain test below;
// `monospace_family` applies it platform-wise.
pub fn monospace_candidates() -> &'static [&'static str] {
    &["Menlo", "Consolas", "DejaVu Sans Mono", "monospace"]
}

/// The platform-appropriate monospace family: the first candidate known to
/// ship on the compiling target OS. Unit tests pin chain properties only.
pub fn monospace_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        // Linux & other unix: DejaVu Sans Mono is the most commonly
        // installed; GPUI's own fallback also honors "monospace".
        "DejaVu Sans Mono"
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The chain must end with a generic `monospace` fallback so no platform
    /// is ever left without a working font, and the platform pick must be a
    /// member of the chain it advertises.
    #[test]
    fn monospace_chain_has_generic_fallback_and_platform_pick_is_member() {
        let candidates = monospace_candidates();
        assert_eq!(candidates.last(), Some(&"monospace"));
        assert!(candidates.contains(&monospace_family()));
        assert!(!monospace_family().is_empty());
    }
}
