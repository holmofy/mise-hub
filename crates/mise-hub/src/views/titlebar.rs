//! Custom window title bar.
//!
//! On macOS the traffic lights stay native: `main.rs` opens the window with
//! `TitleBar::window_options()`, which makes the system title bar transparent
//! and repositions the three buttons into this bar. This module therefore
//! only reserves room for them on the left and puts the language and theme
//! pickers on the right — the app title text is gone entirely.
//!
//! On Linux / Windows the component `TitleBar` draws the app's own window
//! controls; the pickers sit on the same row on every platform so the layout
//! never shifts between them. Dragging the empty area of the bar moves the
//! window and a double click performs the platform title bar action — both
//! are provided by the component `TitleBar` itself.

use crate::app::{LOCALES, RootView};
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Sizable as _, StyledExt as _, TITLE_BAR_HEIGHT, TitleBar};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

/// One entry of a picker menu: `(value written into settings, label shown in
/// the menu)`. Kept as plain data so the tests below can assert what the
/// menus offer without opening a window.
type MenuEntry = (String, String);

/// What a picker does when the user picks a value. `RootView` is borrowed
/// mutably so the shared `apply_*_setting` helpers can persist the choice.
type ApplyPicker = Rc<dyn Fn(&mut RootView, &'static str, &mut Window, &mut Context<RootView>)>;

/// The locale menu entries: every supported locale, each labelled in its own
/// language (never translated — a language list written in another language
/// is unreadable).
fn locale_entries() -> Vec<MenuEntry> {
    LOCALES
        .into_iter()
        .map(|(code, label)| (code.to_string(), label.to_string()))
        .collect()
}

/// The theme menu entries: every registered theme name, used as both the
/// value written into settings and the menu label.
fn theme_entries(theme_names: &[String]) -> Vec<MenuEntry> {
    theme_names
        .iter()
        .map(|name| (name.clone(), name.clone()))
        .collect()
}

/// Render the custom title bar.
///
/// `theme_names` / `current_theme` and `current_locale` are snapshotted by
/// the caller (`RootView::render`) so the builders never read the view
/// entity mid-render. Both pickers go through the shared
/// `RootView::apply_theme_setting` / `RootView::apply_locale_setting` that
/// the settings page uses, so the two entry points cannot drift apart.
pub fn render(
    cx: &mut Context<RootView>,
    theme_names: &[String],
    current_theme: &str,
    current_locale: &str,
) -> impl IntoElement + use<> {
    TitleBar::new()
        .h(TITLE_BAR_HEIGHT)
        // No app title: the window has no system title text either, so the
        // bar stays empty apart from the native buttons and the two pickers.
        .child(div().flex_1())
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .pr_2()
                .child(picker(
                    cx,
                    "titlebar-language-picker",
                    RootView::locale_label(current_locale).to_string(),
                    locale_entries(),
                    current_locale.to_string(),
                    Rc::new(|this, value, window, cx| this.apply_locale_setting(value, window, cx)),
                ))
                .child(picker(
                    cx,
                    "titlebar-theme-picker",
                    current_theme.to_string(),
                    theme_entries(theme_names),
                    current_theme.to_string(),
                    Rc::new(|this, value, window, cx| {
                        this.apply_theme_setting(value.to_string(), window, cx)
                    }),
                )),
        )
}

/// Build one picker button: a menu of `entries` (value, label) with the
/// `current` one checked, whose selection calls `apply`.
///
/// The values are leaked to `'static` because the menu builder runs outside
/// the render call that produced them; the list is at most a handful of
/// short strings (locales and theme names), built once per render.
fn picker(
    cx: &mut Context<RootView>,
    id: &'static str,
    current_label: String,
    entries: Vec<MenuEntry>,
    current: String,
    apply: ApplyPicker,
) -> AnyElement {
    let weak = cx.weak_entity();
    Button::new(id)
        .outline()
        .xsmall()
        .label(current_label)
        .dropdown_menu(move |menu, _, _| {
            entries.iter().fold(menu, |menu, (value, label)| {
                let value: &'static str = Box::leak(value.clone().into_boxed_str());
                let checked = value == current;
                let apply = apply.clone();
                let weak = weak.clone();
                menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(
                    move |_, window, cx| {
                        if let Some(view) = weak.upgrade() {
                            view.update(cx, |this, cx| apply(this, value, window, cx));
                        }
                    },
                ))
            })
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Import only the symbols the tests touch. `use super::*` would pull the
    // GPUI view types into the test module, which blows the macro recursion
    // limit when the `test` harness expands.
    use super::{LOCALES, locale_entries, theme_entries};
    use crate::app::RootView;

    /// The picker labels come from the shared `LOCALES` table: a locale that
    /// is not in the table would render a fallback label that the menu never
    /// offers, so the user could not switch back to it.
    #[test]
    fn locale_label_is_always_a_known_locale() {
        for (code, label) in LOCALES {
            assert_eq!(RootView::locale_label(code), label);
        }
    }

    /// An unknown locale (e.g. a hand-edited settings file) falls back to
    /// English — which IS offered by the picker — never to a blank label.
    #[test]
    fn unknown_locale_label_falls_back_to_a_known_one() {
        let fallback = RootView::locale_label("de");
        assert!(LOCALES.iter().any(|(_, label)| *label == fallback));
    }

    /// The locale menu must offer EVERY supported locale. Dropping one would
    /// leave the picker showing a value the user cannot switch away from.
    #[test]
    fn locale_entries_offer_every_supported_locale() {
        let entries = locale_entries();
        assert_eq!(entries.len(), LOCALES.len());
        for (code, _) in LOCALES {
            assert!(
                entries.iter().any(|(value, _)| value == code),
                "locale menu does not offer {code}"
            );
        }
    }

    /// Locale codes and labels must both be unique: a duplicate would render
    /// two identical rows and make the checked state ambiguous.
    #[test]
    fn locale_entries_have_unique_codes_and_labels() {
        let entries = locale_entries();
        let mut codes: Vec<&str> = entries.iter().map(|(v, _)| v.as_str()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), entries.len(), "duplicate locale code");
        let mut labels: Vec<&str> = entries.iter().map(|(_, l)| l.as_str()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), entries.len(), "duplicate locale label");
    }

    /// No locale entry may carry a blank label: the menu would render an
    /// unreadable row.
    #[test]
    fn locale_entries_have_non_blank_labels() {
        for (value, label) in locale_entries() {
            assert!(!label.trim().is_empty(), "blank label for {value}");
        }
    }

    /// The theme menu mirrors the registry list exactly: a name shown in the
    /// picker that the registry does not know would resolve to the default
    /// light theme (see `theme::apply_theme`), silently ignoring the click.
    #[test]
    fn theme_entries_mirror_the_registered_theme_list() {
        let names = vec!["Default Light".to_string(), "Default Dark".to_string()];
        let entries = theme_entries(&names);
        assert_eq!(entries.len(), names.len());
        for (value, label) in entries {
            assert!(names.contains(&value));
            assert_eq!(value, label, "theme labels are the theme names");
        }
    }

    /// An empty registry (a broken install) yields an empty menu rather than
    /// a bogus entry.
    #[test]
    fn theme_entries_of_an_empty_registry_are_empty() {
        assert!(theme_entries(&[]).is_empty());
    }
}
