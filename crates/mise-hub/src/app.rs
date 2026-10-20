use crate::i18n;
use crate::state::{AppState, Loadable, Selection};
use crate::theme;
use crate::views::{setup, sidebar, tool_detail, tool_list};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mise_core::{AppSettings, ScopeFilter, ToolRow};
use rust_i18n::t;
use std::path::PathBuf;

/// Locale choices exposed by the language picker: (locale code, display label).
const LOCALES: [(&str, &str); 2] = [("en", "English"), ("zh-CN", "中文")];

pub struct RootView {
    pub state: AppState,
    pub settings: AppSettings,
    pub settings_path: PathBuf,
    pub projects_collapsed: bool,
    pub tool_filter: ScopeFilter,
    pub search_input: Entity<InputState>,
    pub notice: Option<String>,
    _search_subscription: Subscription,
}

impl RootView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        settings: AppSettings,
        settings_path: PathBuf,
    ) -> Self {
        let path = mise_core::default_store_path();
        let projects = mise_core::ProjectStore::load(&path).unwrap_or_default();
        let mut state = AppState::new(projects).with_store_path(path.clone());
        state.detect();
        state.ensure_registry();

        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("tools.search_installed").to_string())
        });
        let search_subscription =
            cx.subscribe(&search_input, |_this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });

        let mut this = Self {
            state,
            settings,
            settings_path,
            projects_collapsed: false,
            tool_filter: ScopeFilter::All,
            search_input,
            notice: None,
            _search_subscription: search_subscription,
        };
        // Mark the installed list as loading before the (synchronous) refresh
        // so a slow `mise ls` reads as "loading", not as "no tools installed".
        this.state.installed = Loadable::Loading;
        this.refresh_installed();
        this
    }

    fn locale_label(code: &str) -> &'static str {
        LOCALES
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, label)| *label)
            .unwrap_or("English")
    }

    /// Change the current selection, resetting per-page state and refreshing
    /// installed tools when navigating to a tools page.
    pub(crate) fn select(&mut self, sel: Selection, cx: &mut Context<Self>) {
        self.state.selection = sel;
        self.state.selected_tool = None;
        self.tool_filter = ScopeFilter::All;
        self.notice = None;
        if self.is_tools_page() {
            // Same loading marker as startup: the refresh below is synchronous,
            // but a slow `mise ls` must not be mistaken for an empty list.
            self.state.installed = Loadable::Loading;
            self.refresh_installed();
        }
        cx.notify();
    }

    fn is_tools_page(&self) -> bool {
        matches!(
            self.state.selection,
            Selection::GlobalTools | Selection::ProjectTools(_)
        )
    }

    /// Refresh installed tools, storing failures on the `installed` Loadable.
    fn refresh_installed(&mut self) {
        match self.state.refresh_installed() {
            Ok(()) => {}
            Err(e) => self.state.installed = Loadable::Failed(e.to_string()),
        }
    }

    /// Open a folder picker and add the chosen folder as a project, then select
    /// that project's tools page.
    pub(crate) fn prompt_add_project(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            // rfd's sync dialog runs a nested event loop on the main thread;
            // the foreground executor runs this task on the main thread.
            let picked = rfd::FileDialog::new().pick_folder();
            let Some(path) = picked else { return };
            let id = path
                .canonicalize()
                .unwrap_or_else(|_| path.clone())
                .to_string_lossy()
                .into_owned();
            this.update(cx, |this, cx| match this.state.add_project(&path) {
                Ok(()) => this.select(Selection::ProjectTools(id), cx),
                Err(e) => this.notice = Some(e.to_string()),
            })
            .ok();
        })
        .detach();
    }

    /// Ask for confirmation, then remove a project from the store.
    pub(crate) fn prompt_remove_project(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            // The dialog builder may run again on rebuild, so clone into the
            // per-build `on_ok` closure instead of moving out of this one.
            let weak = weak.clone();
            let id = id.clone();
            let name = name.clone();
            alert
                .confirm()
                .title(t!("nav.confirm_remove_title").to_string())
                .description(format!("{} — {}", name, t!("nav.confirm_remove_body")))
                .ok_text(t!("nav.remove").to_string())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(t!("nav.cancel").to_string())
                .on_ok(move |_, _, cx| {
                    if let Some(view) = weak.upgrade() {
                        view.update(cx, |this, cx| {
                            this.state.remove_project(&id);
                            this.notice = None;
                            cx.notify();
                        });
                    }
                    true
                })
        });
    }

    fn tools_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (filter, show_filter) = match &self.state.selection {
            Selection::GlobalTools => (ScopeFilter::All, false),
            Selection::ProjectTools(_) => (self.tool_filter, true),
            _ => (ScopeFilter::All, false),
        };

        let search_query = self.search_input.read(cx).value().to_string();
        let all_rows = self.state.rows(filter);
        let rows: Vec<ToolRow> = if search_query.is_empty() {
            all_rows
        } else {
            let q = search_query.to_lowercase();
            all_rows
                .into_iter()
                .filter(|r| {
                    r.name.to_lowercase().contains(&q) || r.version.to_lowercase().contains(&q)
                })
                .collect()
        };

        let selected_key = self.state.selected_tool.clone();
        let selected_row = selected_key
            .as_ref()
            .and_then(|key| rows.iter().find(|r| &tool_list::row_key(r) == key))
            .cloned();

        let load_error = match &self.state.installed {
            Loadable::Failed(err) => Some(err.as_str()),
            _ => None,
        };
        let loading = matches!(self.state.installed, Loadable::Loading);
        let list = tool_list::render(
            cx,
            &self.search_input,
            rows,
            selected_key.as_deref(),
            filter,
            show_filter,
            loading,
            load_error,
        );
        let detail = match &selected_row {
            Some(row) => tool_detail::render(row, cx).into_any_element(),
            None => placeholder_detail(cx).into_any_element(),
        };

        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .child(list)
            .child(detail)
    }
}

fn placeholder_detail(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_color(theme.muted_foreground)
        .text_sm()
        .child(t!("tools.select_tool").to_string())
}

fn notice_banner(cx: &App, msg: &str) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .w_full()
        .px_3()
        .py_2()
        .text_sm()
        .text_color(theme.danger)
        .bg(theme.danger.opacity(0.1))
        .child(msg.to_string())
}

impl Render for RootView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.mise.is_none() {
            let on_detect = cx.listener(|this, _event, _window, cx| {
                this.state.detect();
                // Refresh now that mise may be present; otherwise the tools
                // page would sit on the startup `Loading` marker forever.
                this.refresh_installed();
                cx.notify();
            });
            return setup::render(on_detect).into_any_element();
        }

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

        let right_pane = match &self.state.selection {
            Selection::GlobalTools | Selection::ProjectTools(_) => {
                self.tools_pane(cx).into_any_element()
            }
            _ => div()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .text_sm()
                .child(t!("nav.not_available").to_string())
                .into_any_element(),
        };

        // The notice banner renders at the root, above the right pane, on
        // EVERY page (including Discover): a failed add-project does not
        // change the page, so a tools-pane-only banner would never be seen
        // from the default selection (AGENTS.md rule 9: errors must surface).
        let right_col = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when_some(self.notice.clone(), |d, msg| {
                d.child(notice_banner(cx, &msg))
            })
            .child(right_pane);

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
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(t!("app.name").to_string()),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(language_picker)
                            .child(theme_picker),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar::render(
                        cx,
                        &self.state.projects.projects,
                        &self.state.selection,
                        self.projects_collapsed,
                    ))
                    .child(right_col),
            )
            .into_any_element()
    }
}
