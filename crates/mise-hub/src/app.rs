use crate::i18n;
use crate::state::AppState;
use crate::theme;
use crate::views::sidebar;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use mise_core::AppSettings;
use rust_i18n::t;
use std::path::PathBuf;

/// Locale choices exposed by the language picker: (locale code, display label).
const LOCALES: [(&str, &str); 2] = [("en", "English"), ("zh-CN", "中文")];

pub struct RootView {
    pub state: AppState,
    pub settings: AppSettings,
    pub settings_path: PathBuf,
}

impl RootView {
    pub fn new(
        _window: &mut Window,
        _cx: &mut Context<Self>,
        settings: AppSettings,
        settings_path: PathBuf,
    ) -> Self {
        let path = mise_core::default_store_path();
        let projects = mise_core::ProjectStore::load(&path).unwrap_or_default();
        Self {
            state: AppState::new(projects),
            settings,
            settings_path,
        }
    }

    fn locale_label(code: &str) -> &'static str {
        LOCALES
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, label)| *label)
            .unwrap_or("English")
    }
}

impl Render for RootView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_names = theme::theme_names(cx);
        let current_theme = self.settings.theme.clone();
        let current_locale = self.settings.locale.clone();
        let settings_path = self.settings_path.clone();
        let weak = cx.weak_entity();

        // Theme picker: lists every registered theme and applies the selection.
        let theme_picker = {
            let weak = weak.clone();
            let settings_path = settings_path.clone();
            let current = current_theme.clone();
            Button::new("theme-picker")
                .outline()
                .small()
                .label(current.clone())
                .dropdown_menu(move |menu, _, _| {
                    theme_names.iter().cloned().fold(menu, |menu, name| {
                        menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(name == current)
                                .on_click({
                                    let weak = weak.clone();
                                    let settings_path = settings_path.clone();
                                    let name = name.clone();
                                    move |_, window, cx| {
                                        theme::apply_theme(&name, window, cx);
                                        if let Some(view) = weak.upgrade() {
                                            view.update(cx, |this, cx| {
                                                this.settings.theme = name.clone();
                                                let _ = this.settings.save(&settings_path);
                                                cx.notify();
                                            });
                                        }
                                    }
                                }),
                        )
                    })
                })
        };

        // Language picker: switches locale and persists the choice.
        let language_picker = {
            let weak = weak.clone();
            let settings_path = settings_path.clone();
            let current = current_locale.clone();
            Button::new("language-picker")
                .outline()
                .small()
                .label(Self::locale_label(&current))
                .dropdown_menu(move |menu, _, _| {
                    LOCALES.into_iter().fold(menu, |menu, (code, label)| {
                        menu.item(
                            PopupMenuItem::new(label)
                                .checked(code == current.as_str())
                                .on_click({
                                    let weak = weak.clone();
                                    let settings_path = settings_path.clone();
                                    move |_, _window, cx| {
                                        i18n::set_locale(code);
                                        if let Some(view) = weak.upgrade() {
                                            view.update(cx, |this, cx| {
                                                this.settings.locale = code.to_string();
                                                let _ = this.settings.save(&settings_path);
                                                cx.notify();
                                            });
                                        }
                                    }
                                }),
                        )
                    })
                })
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_semibold().child(t!("app.name").to_string()))
                    .child(
                        div().h_flex().gap_2().child(language_picker).child(theme_picker),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar::render(&self.state, cx))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .p_4()
                            .child(format!("{:?}", self.state.selection)),
                    ),
            )
    }
}
