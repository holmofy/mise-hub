use crate::i18n;
use crate::state::{AppState, Loadable, Selection};
use crate::theme;
use crate::views::{discover, setup, sidebar, terminal, tool_detail, tool_list};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mise_core::{AppSettings, ScopeFilter, SearchHit, StreamEvent, ToolRow};
use rust_i18n::t;
use std::collections::HashSet;
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
    /// Discover-page query input; separate from the tools-page input because
    /// the two search boxes filter different lists (with different
    /// placeholders) but can keep their own text while switching pages.
    pub discover_input: Entity<InputState>,
    /// Search results for the discover page, refreshed once per render (query
    /// change or selection change). The virtual-list closure only slices this
    /// vec, so scrolling never re-runs the search.
    pub discover_hits: Vec<SearchHit>,
    /// Track the last discover query to reset the scroll position on a new
    /// query only (not on every unrelated re-render).
    discover_last_query: String,
    /// Scroll position of the discover list, kept on the view so re-renders
    /// (keystrokes) do not jump the list back to the top.
    pub discover_scroll: VirtualListScrollHandle,
    pub notice: Option<String>,
    _search_subscription: Subscription,
    _discover_subscription: Subscription,
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
        // Load the tool registry up front (blocking, like `refresh_installed`
        // below). A failure is stored on `state.registry` and rendered by the
        // discover page with the raw error, so it never becomes a blank page
        // (AGENTS.md rule 9).
        let _ = state.ensure_registry();

        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("tools.search_installed").to_string())
        });
        let search_subscription =
            cx.subscribe(&search_input, |_this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });

        let discover_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("discover.search").to_string()));
        let discover_subscription =
            cx.subscribe(&discover_input, |_this, _input, event: &InputEvent, cx| {
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
            discover_input,
            discover_hits: Vec::new(),
            discover_last_query: String::new(),
            discover_scroll: VirtualListScrollHandle::new(),
            notice: None,
            _search_subscription: search_subscription,
            _discover_subscription: discover_subscription,
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

    /// Pick a tool on the discover page and START loading its remote
    /// versions asynchronously: the blocking `mise ls-remote` call runs on a
    /// background thread; the UI thread only awaits the bridged channel via
    /// `cx.spawn` (never touching the entity off-thread), so the page keeps
    /// rendering while versions load. A failure is surfaced verbatim in the
    /// versions panel (AGENTS.md rule 9), never as a blank area.
    pub(crate) fn select_discover_tool(&mut self, tool: String, cx: &mut Context<Self>) {
        self.state.selected_tool = Some(tool.clone());
        self.state.versions_tool = Some(tool.clone());
        self.state.versions = Loadable::Loading;
        cx.notify();

        let Some(mise) = self.state.mise.clone() else {
            self.state.versions = Loadable::Failed(t!("discover.versions_unavailable").to_string());
            cx.notify();
            return;
        };
        let (tx, rx) = async_channel::unbounded();
        let thread_tool = tool.clone();
        std::thread::spawn(move || {
            let result = mise.remote_versions(&thread_tool, None);
            tx.send_blocking(result).ok();
        });
        let expected_tool = tool;
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                this.update(cx, |this, cx| {
                    // Only apply while this result is still the picked tool
                    // (a stale fetch must not clobber a newer selection).
                    if this.state.versions_tool.as_deref() == Some(&expected_tool) {
                        this.state.versions = match result {
                            Ok(v) => Loadable::Ready(v),
                            Err(e) => Loadable::Failed(e.to_string()),
                        };
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Start an install (install ONLY, never `mise use` — AGENTS.md rule 4)
    /// and stream its output into a new terminal session. The std receiver
    /// from core is bridged onto an async channel by a background thread —
    /// the UI thread only awaits, never touching the entity off-thread.
    pub(crate) fn run_install(&mut self, tool: &str, version: &str, cx: &mut Context<Self>) {
        let rx = match self.state.start_install(tool, version) {
            Ok(rx) => rx,
            Err(e) => {
                // Surface the raw error inside its own terminal tab, as a
                // failed run (AGENTS.md rule 9), not a silent nothing.
                self.state
                    .terminal
                    .begin(format!("install {tool}@{version}"));
                self.state
                    .terminal
                    .push_event(StreamEvent::Stderr(e.to_string()));
                self.state.terminal.push_event(StreamEvent::Exited(-1));
                cx.notify();
                return;
            }
        };

        let (tx, async_rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            for event in rx {
                if tx.send_blocking(event).is_err() {
                    break;
                }
            }
        });

        cx.spawn(async move |this, cx| {
            while let Ok(event) = async_rx.recv().await {
                let done = matches!(event, StreamEvent::Exited(_));
                this.update(cx, |this, cx| {
                    this.state.terminal.push_event(event.clone());
                    if done {
                        // Refresh only on completion: rule 4 — after the
                        // install finished the panel can honestly show
                        // 「已安装 · 未生效」 with the apply actions exposed.
                        this.refresh_installed();
                    }
                    cx.notify();
                })
                .ok();
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    /// Save the active terminal session's text through an rfd save dialog.
    /// The dialog runs inside the foreground async task (main thread), like
    /// `prompt_add_project`; the file write happens after the pick.
    pub(crate) fn save_terminal_log(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let Some(path) = rfd::FileDialog::new()
                .set_file_name("mise-install.log")
                .save_file()
            else {
                return;
            };
            this.update(cx, |this, cx| {
                let Some(text) = this.state.terminal.active_text() else {
                    return;
                };
                match std::fs::write(&path, text) {
                    Ok(()) => this.notice = None,
                    Err(e) => this.notice = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Copy the active session's text to the clipboard.
    pub(crate) fn copy_terminal(&self, cx: &App) {
        if let Some(text) = self.state.terminal.active_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Compute the discover-page search/browse results once per render. The
    /// virtual list only slices `discover_hits` afterwards, so scrolling does
    /// not re-run the search, and the index itself is cached in `AppState`
    /// (built once when the registry loaded).
    fn discover_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.discover_input.read(cx).value().to_string();
        // A new query starts the list at the top; unrelated re-renders keep
        // the scroll position.
        if query != self.discover_last_query {
            self.discover_last_query = query.clone();
            self.discover_scroll.set_offset(point(px(0.), px(0.)));
        }
        // No limit: browse mode (empty query) must show the whole registry.
        self.discover_hits = self.state.search(&query, usize::MAX);

        let installed_names: HashSet<String> = self
            .state
            .installed
            .ready()
            .map(|rows| rows.iter().map(|r| r.name.clone()).collect())
            .unwrap_or_default();
        // Version shown as 「已安装 · 未生效」 for the picked tool (rule 4).
        let not_active_version = self
            .state
            .versions_tool
            .as_deref()
            .and_then(|tool| self.state.installed_but_not_active(tool));
        discover::render(
            cx,
            &self.discover_input,
            &self.discover_hits,
            &installed_names,
            &self.discover_scroll,
            &self.state.registry,
            &self.state.versions,
            self.state.versions_tool.as_deref(),
            not_active_version,
        )
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
                // mise may have appeared only now (setup page path): the
                // startup load was a no-op back then. Failures stay in
                // `state.registry` and surface on the discover page.
                let _ = this.state.ensure_registry();
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
            Selection::Discover => self.discover_pane(cx).into_any_element(),
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

        // Bottom terminal panel: only meaningful once a run exists — before
        // that the layout stays undiscovered-clean.
        let has_sessions = !self.state.terminal.sessions.is_empty();

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
            .when(has_sessions, |root| {
                root.child(terminal::render(&self.state.terminal, cx))
            })
            .into_any_element()
    }
}
