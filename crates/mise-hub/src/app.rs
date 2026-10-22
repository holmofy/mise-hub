use crate::i18n;
use crate::state::{
    AppState, ApplyFlow, EnvTab, Loadable, Selection, resolved_env_name, should_start_task_run,
};
use crate::theme;
use crate::views::{
    discover, env, settings, setup, sidebar, tasks, terminal, titlebar, tool_detail, tool_list,
    upgrade,
};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mise_core::{AppSettings, CoreError, ScopeFilter, SearchHit, StreamEvent, TaskGraph, ToolRow};
use rust_i18n::t;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

/// Locale choices exposed by the language pickers (header AND settings
/// page): (locale code, display label).
pub(crate) const LOCALES: [(&str, &str); 2] = [("en", "English"), ("zh-CN", "中文")];

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
    /// The 「包含超出配置范围的版本（--bump）」 toggle on the upgrade page.
    pub upgrade_bump: bool,
    /// One open 「应用版本」 flow: spec, scope, pin choice and the async
    /// `mise use --dry-run` preview the confirm dialog renders. See
    /// `ApplyFlow` — the dialog reads this on every rebuild, so the validator
    /// stays honest about what will be written before anything is written.
    pub apply_flow: Option<ApplyFlow>,
    /// Environment-page tab (声明 / 生效 / 差异). Defaults to 声明.
    pub env_tab: EnvTab,
    /// Global 「显示全部」 reveal toggle for sensitive values (default OFF —
    /// the mask is the contract; see `env::is_revealed`).
    pub env_show_all: bool,
    /// Per-row reveal set: names whose values the user explicitly revealed.
    pub env_revealed: HashSet<String>,
    /// Per-row expansion set: names whose override chain (other scope's
    /// declaration + effective value) is shown.
    pub env_expanded: HashSet<String>,
    /// 生效 tab filter input (separate entity; the tabs keep their text).
    pub env_filter_input: Entity<InputState>,
    /// Tasks-page filter input (its own entity; the query text survives
    /// page switches like the other list filters do).
    pub tasks_filter_input: Entity<InputState>,
    /// One open 「新增 / 编辑变量」 dialog: its input entities (created ONCE
    /// before opening — dialog rebuilds must not lose editing state, the
    /// gpui-kit docs insist), the name being edited (None = adding), and the
    /// chosen write scope.
    pub env_dialog: Option<EnvDialogState>,
    /// Subscriptions of the open env dialog's inputs (keyboard input must
    /// revalidate the preview / inline error, i.e. trigger a redraw).
    _env_dialog_subscriptions: Vec<Subscription>,
    _search_subscription: Subscription,
    _discover_subscription: Subscription,
    /// Keyboard events of the tasks-page filter input must trigger redraws.
    _tasks_filter_subscription: Subscription,
    /// Settings-page update manifest URL draft (M5-T2). Its value may differ
    /// from `settings.update_manifest_url` until 「保存」 is clicked; the
    /// check button operates on the draft so the user always checks exactly
    /// what is in the box.
    pub update_url_input: Entity<InputState>,
    /// Keyboard input in the URL draft must trigger a redraw (the check
    /// button's enabled state depends on it).
    _update_url_subscription: Subscription,
}

/// Local state of the open 「新增 / 编辑变量」 dialog. The input entities are
/// created before the dialog opens and captured here; the dialog builder
/// (env::edit_dialog_body) reads them through the weak entity so the
/// preview command and inline error stay live while typing.
pub struct EnvDialogState {
    pub name_input: Entity<InputState>,
    pub value_input: Entity<InputState>,
    /// The variable being edited, `None` when adding a new one. While
    /// editing, the name is fixed (rename = unset + set, out of scope here).
    pub editing: Option<String>,
    /// The declaration's OWN source file while editing — the write targets
    /// this exact file via `mise set --file`, never the page's default scope
    /// file. `None` while adding (the scope toggle decides) or when the
    /// declaration has no recoverable source (page scope, stated as such).
    pub editing_source: Option<String>,
    /// `true` = write the global config (`mise set --global`), `false` =
    /// write the CURRENT PROJECT's config (`mise -C <root> set`).
    pub scope_global: bool,
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
        if let Err(e) = state.ensure_registry() {
            state.note_untrusted(&e);
        }

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

        let env_filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("env.filter").to_string()));
        let env_filter_subscription = cx.subscribe(
            &env_filter_input,
            |_this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );

        let tasks_filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("tasks.filter").to_string()));
        let tasks_filter_subscription = cx.subscribe(
            &tasks_filter_input,
            |_this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );

        // Settings page (M5-T2): manifest URL draft, prefilled with the
        // persisted value so the field never starts blank after a restart.
        let update_url_input = cx.new(|cx| {
            let state = InputState::new(window, cx)
                .placeholder(t!("settings.update_url_placeholder").to_string());
            if settings.update_manifest_url.is_empty() {
                state
            } else {
                state.default_value(settings.update_manifest_url.clone())
            }
        });
        let update_url_subscription = cx.subscribe(
            &update_url_input,
            |_this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );

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
            upgrade_bump: false,
            apply_flow: None,
            env_tab: EnvTab::Declarations,
            env_show_all: false,
            env_revealed: HashSet::new(),
            env_expanded: HashSet::new(),
            env_filter_input,
            tasks_filter_input,
            env_dialog: None,
            _env_dialog_subscriptions: vec![env_filter_subscription],
            _search_subscription: search_subscription,
            _discover_subscription: discover_subscription,
            _tasks_filter_subscription: tasks_filter_subscription,
            update_url_input,
            _update_url_subscription: update_url_subscription,
        };
        // Mark the installed list as loading before the (synchronous) refresh
        // so a slow `mise ls` reads as "loading", not as "no tools installed".
        this.state.installed = Loadable::Loading;
        this.refresh_installed();
        this
    }

    pub(crate) fn locale_label(code: &str) -> &'static str {
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
        // A leaving page's tool detail and apply dialog data must not bleed
        // into the next one; the async loader guards drop them too.
        self.state.tool_info = Loadable::Idle;
        self.state.tool_info_tool = None;
        self.apply_flow = None;
        if self.is_tools_page() {
            // Same loading marker as startup: the refresh below is synchronous,
            // but a slow `mise ls` must not be mistaken for an empty list.
            self.state.installed = Loadable::Loading;
            self.refresh_installed();
            // Per-tool upgrade actions (spec §6.3) need the outdated list
            // for THIS page's scope — not a stale snapshot from another page
            // (scope consistency), so it is loaded here too.
            self.load_outdated_for_selection(cx);
        }
        if matches!(self.state.selection, Selection::Upgrade) {
            self.load_outdated_for_selection(cx);
        }
        // Tasks pages: reset the detail selection (a task of the previous
        // page must not stay "selected" on the next one) and load the list.
        if matches!(
            self.state.selection,
            Selection::GlobalTasks | Selection::ProjectTasks(_)
        ) {
            self.state.selected_task = None;
            self.load_tasks_for_selection(cx);
        }
        // Env pages: reset the per-page view state and load both scopes'
        // data for the diff view; values of another scope must not bleed.
        if matches!(
            self.state.selection,
            Selection::GlobalEnv | Selection::ProjectEnv(_)
        ) {
            self.env_tab = EnvTab::Declarations;
            self.env_show_all = false;
            self.env_revealed.clear();
            self.env_expanded.clear();
            self.state.env_exact.clear();
            self.load_env_for_selection(false, cx);
        }
        // Settings page: probe mise capabilities on the FIRST visit (and
        // after a manual re-probe reset them); probes run on a background
        // thread so a slow `ls-remote` never blocks the UI thread.
        if matches!(self.state.selection, Selection::Settings)
            && matches!(self.state.diagnostics, Loadable::Idle)
        {
            self.rerun_diagnostics(cx);
        }
        cx.notify();
    }

    // -------------------------------------------------------------------
    // Settings page (M5-T2)
    // -------------------------------------------------------------------

    /// Apply a theme and persist the choice. Shared by the header picker
    /// and the settings page so the two entry points cannot drift apart.
    pub(crate) fn apply_theme_setting(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        theme::apply_theme(&name, window, cx);
        self.settings.theme = name;
        let _ = self.settings.save(&self.settings_path);
        cx.notify();
    }

    /// Switch the locale and persist the choice. Shared by the header
    /// picker and the settings page; the search placeholders were baked
    /// into their InputStates at startup, so they are re-evaluated here to
    /// follow the switch without a restart.
    pub(crate) fn apply_locale_setting(
        &mut self,
        code: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        i18n::set_locale(code);
        self.settings.locale = code.to_string();
        let _ = self.settings.save(&self.settings_path);
        // Every InputState was created with a placeholder baked in at
        // startup, so all of them are re-evaluated here — otherwise half
        // the filters would keep the old language until a restart.
        let search_input = self.search_input.clone();
        let discover_input = self.discover_input.clone();
        let env_filter_input = self.env_filter_input.clone();
        let tasks_filter_input = self.tasks_filter_input.clone();
        search_input.update(cx, |input, cx| {
            input.set_placeholder(t!("tools.search_installed").to_string(), window, cx)
        });
        discover_input.update(cx, |input, cx| {
            input.set_placeholder(t!("discover.search").to_string(), window, cx)
        });
        env_filter_input.update(cx, |input, cx| {
            input.set_placeholder(t!("env.filter").to_string(), window, cx)
        });
        tasks_filter_input.update(cx, |input, cx| {
            input.set_placeholder(t!("tasks.filter").to_string(), window, cx)
        });
        let update_url_input = self.update_url_input.clone();
        update_url_input.update(cx, |input, cx| {
            input.set_placeholder(
                t!("settings.update_url_placeholder").to_string(),
                window,
                cx,
            )
        });
        cx.notify();
    }

    /// Re-run the capability probes on a background thread. Guarded by
    /// `diagnostics_in_flight` so rapid clicks cannot stack probe runs;
    /// the result replaces the snapshot unconditionally (probes are not
    /// scope-dependent, so there is no stale-scope guard here).
    pub(crate) fn rerun_diagnostics(&mut self, cx: &mut Context<Self>) {
        if self.state.diagnostics_in_flight {
            return;
        }
        let Some(mise) = self.state.mise.clone() else {
            return;
        };
        self.state.diagnostics_in_flight = true;
        self.state.diagnostics = Loadable::Loading;
        cx.notify();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            tx.send_blocking(mise.diagnostics()).ok();
        });
        cx.spawn(async move |this, cx| {
            let result = rx.recv().await;
            this.update(cx, |this, cx| {
                // The in-flight flag is cleared in BOTH branches: a worker
                // thread that panicked (or a dropped sender) must not leave
                // the 「重新探测」 button disabled for the rest of the session.
                this.state.diagnostics_in_flight = false;
                this.state.diagnostics = match result {
                    Ok(report) => Loadable::Ready(report),
                    // The worker died without an answer — say so instead of
                    // leaving the page on "Probing…" forever (rule 9).
                    Err(_) => {
                        Loadable::Failed(t!("settings.diagnostics_worker_failed").to_string())
                    }
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Check the manifest at `url` on a background thread (the fetch
    /// blocks; the UI thread never does). The result compares against this
    /// app's own version — mise-hub only ever TELLS the user about a new
    /// release, it never downloads or replaces itself.
    pub(crate) fn check_for_updates(&mut self, url: String, cx: &mut Context<Self>) {
        if !AppState::update_check_enabled(&url) || self.state.update_in_flight {
            return;
        }
        self.state.update_in_flight = true;
        self.state.update = Loadable::Loading;
        cx.notify();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            let result = mise_core::fetch_manifest(&url, std::time::Duration::from_secs(10))
                .and_then(|body| mise_core::check_update(|| Ok(body), mise_core::version()));
            tx.send_blocking(result).ok();
        });
        cx.spawn(async move |this, cx| {
            let result = rx.recv().await;
            this.update(cx, |this, cx| {
                // Cleared in both branches — a dead worker must not leave
                // the check button permanently disabled.
                this.state.update_in_flight = false;
                this.state.update = match result {
                    Ok(Ok(check)) => Loadable::Ready(check),
                    Ok(Err(e)) => Loadable::Failed(e.to_string()),
                    // The worker died without an answer — say so instead of
                    // leaving the page on "Checking…" forever (rule 9).
                    Err(_) => Loadable::Failed(t!("settings.update_worker_failed").to_string()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Persist the URL draft from the settings-page input into
    /// `AppSettings`. Writes only the app's own settings file — never a
    /// mise config (AGENTS.md rule 3 concerns mise files only).
    pub(crate) fn save_update_url(&mut self, cx: &mut Context<Self>) {
        let url = self.update_url_input.read(cx).value().trim().to_string();
        self.settings.update_manifest_url = url;
        let _ = self.settings.save(&self.settings_path);
        cx.notify();
    }

    /// Hand the release page to the platform browser. Only http(s) URLs
    /// ever reach the OS opener (`opener::open_url` refuses the rest); a
    /// failure surfaces as the root notice with the raw error (rule 9).
    pub(crate) fn open_update_page(&mut self, url: String, cx: &mut Context<Self>) {
        self.notice = crate::opener::open_url(&url)
            .err()
            .map(|e| format!("{}: {e}", t!("settings.update_open_failed")));
        cx.notify();
    }

    fn is_tools_page(&self) -> bool {
        matches!(
            self.state.selection,
            Selection::GlobalTools | Selection::ProjectTools(_)
        )
    }

    /// Refresh installed tools, storing failures on the `installed` Loadable.
    pub(crate) fn refresh_installed(&mut self) {
        match self.state.refresh_installed() {
            Ok(()) => {}
            Err(e) => {
                self.state.note_untrusted(&e);
                self.state.installed = Loadable::Failed(e.to_string());
            }
        }
    }

    /// Refresh the outdated list, keeping the raw error in `Loadable::Failed`
    /// (AGENTS.md rule 9). Setting `Loading` first is the caller's job (the
    /// pane or `select`), mirroring `installed`. Used only as the post-run
    /// refresh (the result lands synchronously after the run already ended,
    /// so blocking a tokio-free UI thread for the tail of an upgrade run is
    /// acceptable there); page navigation uses `load_outdated_for_selection`.
    pub(crate) fn refresh_outdated(&mut self) {
        match self.state.refresh_outdated() {
            Ok(()) => {}
            Err(e) => {
                self.state.note_untrusted(&e);
                self.state.outdated = Loadable::Failed(e.to_string());
            }
        }
    }

    /// The tools page and the upgrade page BOTH need `mise outdated`
    /// for their current scope (the upgrade page to list rows, the tools page
    /// to show the per-tool upgrade action — spec §6.3). The blocking call
    /// runs on a background thread (bridge pattern of `select_discover_tool`);
    /// the result is stored only while the selection's scope is still the one
    /// it was fetched for, so a slow global listing can never fill a project
    /// page (scope consistency).
    pub(crate) fn load_outdated_for_selection(&mut self, cx: &mut Context<Self>) {
        let expected = match self.state.outdated_scope_key() {
            Some(key) => key,
            None => return,
        };
        // Coalesce via the pure predicate: reuse only a Ready snapshot of the
        // same scope, or an in-flight fetch for the SAME scope (tracked in
        // `outdated_in_flight`, never `outdated_scope` — the completed
        // snapshot's scope and the running fetch's scope are unrelated when
        // the user switches pages mid-fetch, the B → A → B regression).
        if !AppState::should_load_outdated(
            self.state.outdated_in_flight.as_deref(),
            self.state.outdated_scope.as_deref(),
            matches!(self.state.outdated, Loadable::Ready(_)),
            &expected,
        ) {
            return;
        }
        let Some(mise) = self.state.mise.clone() else {
            return;
        };
        self.state.outdated = Loadable::Loading;
        // Mark the in-flight fetch's scope BEFORE spawning: `outdated_scope`
        // stays attached to the last COMPLETED snapshot and must not be
        // confused with the request currently running.
        self.state.outdated_in_flight = Some(expected.clone());
        cx.notify();

        let root = self.state.current_project_root();
        let global = root.is_none();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            let result = mise.outdated(global, root.as_deref());
            tx.send_blocking(result).ok();
        });
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                this.update(cx, |this, cx| {
                    // Clear the in-flight marker for THIS fetch only; a
                    // newer fetch may already have overwritten it.
                    if this.state.outdated_in_flight.as_deref() == Some(expected.as_str()) {
                        this.state.outdated_in_flight = None;
                    }
                    // Stale-result guard: only apply while the selection's
                    // scope is still the one this fetch was started for.
                    if this.state.outdated_scope_key().as_deref() == Some(expected.as_str()) {
                        this.state.outdated = match result {
                            Ok(tools) => Loadable::Ready(tools),
                            Err(e) => {
                                this.state.note_untrusted(&e);
                                Loadable::Failed(e.to_string())
                            }
                        };
                        this.state.outdated_scope = Some(expected);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    // -------------------------------------------------------------------
    // Tasks page (M4-T2 / M4-T3)
    // -------------------------------------------------------------------

    /// Load `mise tasks ls --json` for the CURRENT scope: the project root
    /// on a project tasks page, global scope otherwise. The blocking call
    /// runs on a background thread; the result is applied only while the
    /// selection's scope is still the one the fetch was started for (the
    /// same stale-guard pattern as `load_outdated_for_selection`). A
    /// failure keeps the raw error in `Loadable::Failed` (rule 9) and feeds
    /// the trust banner.
    pub(crate) fn load_tasks_for_selection(&mut self, cx: &mut Context<Self>) {
        let Some(expected) = self.state.tasks_scope_key() else {
            return;
        };
        // Coalesce via the shared pure predicate: reuse a Ready snapshot of
        // the same scope, or an in-flight fetch for the SAME scope.
        if !AppState::should_load_outdated(
            self.state.tasks_in_flight.as_deref(),
            self.state.tasks_scope.as_deref(),
            matches!(self.state.tasks, Loadable::Ready(_)),
            &expected,
        ) {
            return;
        }
        let Some(mise) = self.state.mise.clone() else {
            return;
        };
        self.state.tasks_in_flight = Some(expected.clone());
        // A slow listing must not read as "no tasks": mark loading first.
        self.state.tasks = Loadable::Loading;
        cx.notify();

        let root = self.state.current_project_root();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            let result = mise.tasks(root.as_deref());
            tx.send_blocking(result).ok();
        });

        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                this.update(cx, |this, cx| {
                    if this.state.tasks_in_flight.as_deref() == Some(expected.as_str()) {
                        this.state.tasks_in_flight = None;
                    }
                    // Stale-result guard: apply only while the selection's
                    // scope is still the one this fetch was started for.
                    if this.state.tasks_scope_key().as_deref() == Some(expected.as_str()) {
                        this.state.tasks = match result {
                            Ok(rows) => Loadable::Ready(rows),
                            Err(e) => {
                                this.state.note_untrusted(&e);
                                Loadable::Failed(e.to_string())
                            }
                        };
                        this.state.tasks_scope = Some(expected.clone());
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Stream a task run (or `mise watch`) into a NEW terminal session.
    /// The terminal core behavior covers the failure display: non-zero
    /// exits land as Failed(code) with stderr rendered raw (rule 9), and
    /// spawn failures open an error session instead of failing silently.
    pub(crate) fn run_task(&mut self, task: &str, watch: bool, cx: &mut Context<Self>) {
        let task = task.to_string();
        let kind = if watch { "task-watch" } else { "task-run" };
        // Hard re-entry guard: never spawn a second `mise watch` for a task
        // that already has a live one (the UI disables the button, but this
        // guard must hold even if the UI guard is bypassed).
        if !should_start_task_run(watch, self.state.task_watch_running(&task)) {
            return;
        }
        self.state.begin_run(kind, &task);
        let (session, rx) = match self.state.start_task_run(&task, watch) {
            Ok(pair) => pair,
            Err(e) => {
                // Release the in-flight key the failed spawn took, and
                // surface the raw error in its own terminal tab.
                self.state.finish_run(kind, &task);
                // The spawn failed BEFORE any watch session was registered,
                // so there is no watch bookkeeping to clean up here.
                let verb = if watch { "watch" } else { "run" };
                let session = self.state.terminal.begin(format!("{verb} {task}"));
                self.state
                    .terminal
                    .push_event_to(session, StreamEvent::Stderr(e.to_string()));
                self.state
                    .terminal
                    .push_event_to(session, StreamEvent::Exited(-1));
                cx.notify();
                return;
            }
        };
        let done_task = task.clone();
        self.stream_to_terminal(
            session,
            rx,
            move |this, cx| {
                this.state.finish_run(kind, &done_task);
                // A watch that exited is no longer 监听中; the session-index
                // guard makes this idempotent: a stale run's cleanup must
                // never erase a NEWER watch's bookkeeping for the same task.
                this.state.finish_task_watch(&done_task, session);
                cx.notify();
            },
            cx,
        );
    }

    /// Snapshot the tasks-page state once per render for the view.
    fn tasks_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let graph: Option<TaskGraph> = self.state.current_task_graph();
        let selected = self.state.selected_task.clone();
        let run_busy = selected
            .as_deref()
            .is_some_and(|name| self.state.is_busy("task-run", name));
        let watch_running = selected
            .as_deref()
            .is_some_and(|name| self.state.task_watch_running(name));
        tasks::render(
            cx,
            &self.tasks_filter_input,
            self.state.tasks_show_hidden,
            &self.state.tasks,
            graph.as_ref(),
            selected.as_deref(),
            run_busy,
            watch_running,
        )
    }

    /// Pick a tool on the discover page and START loading its remote
    /// versions asynchronously: the blocking `mise ls-remote` call runs on a
    /// background thread; the UI thread only awaits the bridged channel via
    /// `cx.spawn` (never touching the entity off-thread), so the page keeps
    /// rendering while versions load. A failure is surfaced verbatim in the
    /// versions panel (AGENTS.md rule 9), never as a blank area.
    pub(crate) fn select_discover_tool(&mut self, tool: String, cx: &mut Context<Self>) {
        // Coalesce repeat requests (AGENTS.md: 同一工具的重复请求要合并):
        // a repeated click on the SAME tool must never spawn another
        // `mise ls-remote`. Reuse an in-flight Loading or an already-Ready
        // result; only a Failed result refetches on a new click.
        if self.state.versions_tool.as_deref() == Some(tool.as_str())
            && matches!(self.state.versions, Loadable::Loading | Loadable::Ready(_))
        {
            self.state.selected_tool = Some(tool);
            cx.notify();
            return;
        }

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

    /// Load `mise tool <name> --json` for the tools page asynchronously,
    /// exactly like `select_discover_tool`: the blocking call runs on a
    /// background thread, the UI thread only awaits the bridged channel, and
    /// a stale result for a differently-selected tool is dropped. A failure
    /// keeps the raw error (`Loadable::Failed`) so the detail pane renders it
    /// while the basic `mise ls` fields stay visible (AGENTS.md rule 9).
    pub(crate) fn load_tool_info(&mut self, tool: String, cx: &mut Context<Self>) {
        // Coalesce: the same tool already Loading/Ready means a fetch is
        // (was just) running; only a Failed result refetches on reselect.
        if self.state.tool_info_tool.as_deref() == Some(tool.as_str())
            && matches!(self.state.tool_info, Loadable::Loading | Loadable::Ready(_))
        {
            return;
        }
        self.fetch_tool_info(tool, cx);
    }

    /// Re-fetch the info of the tool currently shown in the detail pane,
    /// used after an apply / uninstall / upgrade finished: `config_source`
    /// and `installed_versions` would otherwise go stale. A success result
    /// for a tool that is no longer the visible row is dropped by the
    /// render-side staleness check.
    pub(crate) fn refresh_tool_info(&mut self, cx: &mut Context<Self>) {
        if let Some(tool) = self.state.tool_info_tool.clone() {
            self.fetch_tool_info(tool, cx);
        }
    }

    /// Start the `mise tool <name> --json` fetch (no coalescing — the
    /// callers own that decision).
    fn fetch_tool_info(&mut self, tool: String, cx: &mut Context<Self>) {
        self.state.tool_info_tool = Some(tool.clone());
        self.state.tool_info = Loadable::Loading;
        cx.notify();

        let Some(mise) = self.state.mise.clone() else {
            self.state.tool_info = Loadable::Failed(t!("tools.info_unavailable").to_string());
            cx.notify();
            return;
        };
        let root = self.state.current_project_root();
        let (tx, rx) = async_channel::unbounded();
        let thread_tool = tool.clone();
        std::thread::spawn(move || {
            let result = mise.tool_info(&thread_tool, root.as_deref());
            tx.send_blocking(result).ok();
        });
        let expected_tool = tool;
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                this.update(cx, |this, cx| {
                    // Stale-result guard: only apply while this tool is still
                    // the selected one.
                    if this.state.tool_info_tool.as_deref() == Some(&expected_tool) {
                        this.state.tool_info = match result {
                            Ok(info) => Loadable::Ready(info),
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

    /// Open the 「应用版本」 confirm dialog for `spec` in the given scope and
    /// START the async `mise use --dry-run` preview that fills it. The write
    /// (`mise use`) happens ONLY if the user confirms the dialog afterwards
    /// (AGENTS.md rule 4 + rule 10: preview and impact shown before acting).
    pub(crate) fn prompt_apply(
        &mut self,
        spec: String,
        global: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_flow = Some(ApplyFlow {
            spec: spec.clone(),
            global,
            pin: false,
            preview: Loadable::Loading,
        });
        cx.notify();

        // The preview is a blocking subprocess — background thread, bridged
        // back through an async channel (UI thread never blocks on IO).
        if let Some(mise) = self.state.mise.clone() {
            let (g, root) = self.state.use_scope(global);
            let (tx, rx) = async_channel::unbounded();
            let thread_spec = spec.clone();
            let mise_ref = mise;
            std::thread::spawn(move || {
                let result =
                    AppState::use_preview(mise_ref.as_ref(), &thread_spec, g, root.as_deref());
                tx.send_blocking(result).ok();
            });
            let expected = spec.clone();
            cx.spawn(async move |this, cx| {
                if let Ok(result) = rx.recv().await {
                    this.update(cx, |this, cx| {
                        // Only fill a dialog still open for THIS spec.
                        if let Some(flow) =
                            &mut this.apply_flow.iter_mut().find(|f| f.spec == expected)
                        {
                            flow.preview = match result {
                                Ok(p) => Loadable::Ready(p),
                                Err(e) => {
                                    this.state.note_untrusted(&e);
                                    Loadable::Failed(e.to_string())
                                }
                            };
                            cx.notify();
                        }
                    })
                    .ok();
                }
            })
            .detach();
        } else {
            if let Some(flow) = &mut self.apply_flow {
                flow.preview = Loadable::Failed(t!("tools.apply_unavailable").to_string());
            }
            cx.notify();
        }

        // Open the dialog NOW so it renders its loading state while the
        // preview streams in; the content builder re-runs on every redraw
        // and reads the live flow through the weak entity, so the preview
        // appears in place and the pin toggle stays reactive.
        let weak = cx.weak_entity();
        let expected = spec.clone();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let weak = weak.clone();
            alert
                .confirm()
                .title(t!("tools.apply_confirm_title").to_string())
                .description({
                    // Built fresh on every dialog rebuild: this reads the
                    // LIVE flow (preview fills in asynchronously inside the
                    // open dialog), not a frozen snapshot.
                    let weak = weak.clone();
                    tool_detail::apply_dialog_body(weak, cx)
                })
                .ok_text(t!("tools.apply_ok").to_string())
                .cancel_text(t!("nav.cancel").to_string())
                .on_ok({
                    // The dialog builder re-runs on every rebuild, so each
                    // build needs its own copy of the captured spec (same
                    // pattern as `prompt_remove_project`'s on_ok).
                    let expected = expected.clone();
                    move |_, _, cx| {
                        if let Some(view) = weak.upgrade() {
                            view.update(cx, |this, cx| {
                                // Only the flow this dialog was opened for
                                // may proceed: if a different dialog is
                                // open now, OK here writes nothing.
                                let is_this_dialog =
                                    this.apply_flow.as_ref().is_some_and(|f| f.spec == expected);
                                // Only the flow this dialog was opened for
                                // may proceed: a mismatched or replayed OK
                                // writes nothing.
                                if is_this_dialog && let Some(flow) = this.apply_flow.take() {
                                    // Only a ready preview may proceed:
                                    // a failed or still-loading preview
                                    // closes with nothing written.
                                    if matches!(flow.preview, Loadable::Ready(_)) {
                                        this.run_use(&flow.spec, flow.global, flow.pin, cx);
                                    }
                                }
                                cx.notify();
                            });
                        }
                        true
                    }
                })
        });
    }

    /// Stream `mise use [--pin] <spec>` into a NEW terminal session after the
    /// user confirmed. `begin_run` blocks the action buttons for the spec
    /// while it streams; on `Exited` the installed list and the outdated
    /// list are refreshed (the config change can move activation and what
    /// counts as outdated) and `apply_status_key` then shows 已生效.
    pub(crate) fn run_use(&mut self, spec: &str, global: bool, pin: bool, cx: &mut Context<Self>) {
        let spec = spec.to_string();
        self.state.begin_run("use", &spec);
        let (session, rx) = match self.state.start_use(&spec, global, pin) {
            Ok(pair) => pair,
            Err(e) => {
                self.state.note_untrusted(&e);
                self.spawn_error_session(spec, "use", e, cx);
                return;
            }
        };
        let done_spec = spec.clone();
        self.stream_to_terminal(
            session,
            rx,
            move |this, cx| {
                this.state.finish_run("use", &done_spec);
                this.state.outdated = Loadable::Loading;
                this.refresh_outdated();
                this.refresh_installed();
                // Applied versions/config move: re-enrich the detail pane
                // (config source + installed versions must not go stale).
                this.refresh_tool_info(cx);
            },
            cx,
        );
    }

    /// Ask for confirmation, then stream `mise uninstall <tool>@<version>`.
    /// Rule 3: the uninstall NEVER touches any config file — the dialog
    /// states the exact `tool@version` and that the config entry stays.
    pub(crate) fn prompt_uninstall(
        &mut self,
        tool: String,
        version: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            let tool = tool.clone();
            let version = version.clone();
            alert
                .confirm()
                .title(t!("tools.uninstall_confirm_title").to_string())
                .description(
                    t!(
                        "tools.uninstall_confirm_body",
                        spec = format!("{tool}@{version}")
                    )
                    .to_string(),
                )
                .ok_text(t!("tools.uninstall").to_string())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(t!("nav.cancel").to_string())
                .on_ok(move |_, _, cx| {
                    if let Some(view) = weak.upgrade() {
                        view.update(cx, |this, cx| this.run_uninstall(&tool, &version, cx));
                    }
                    true
                })
        });
    }

    /// Stream `mise uninstall` and open its terminal session, mirroring
    /// `run_install`. Uninstall only removes installed files; the config is
    /// untouched (rule 3), so only the installed list is refreshed on exit.
    pub(crate) fn run_uninstall(&mut self, tool: &str, version: &str, cx: &mut Context<Self>) {
        let spec = crate::state::uninstall_spec(tool, version);
        self.state.begin_run("uninstall", &spec);
        let (session, rx) = match self.state.start_uninstall(tool, version) {
            Ok(pair) => pair,
            Err(e) => {
                self.spawn_error_session(spec, "uninstall", e, cx);
                return;
            }
        };
        let done_spec = spec;
        self.stream_to_terminal(
            session,
            rx,
            move |this, cx| {
                this.state.finish_run("uninstall", &done_spec);
                this.refresh_installed();
                // The uninstalled version disappears from
                // `installed_versions` in the detail pane.
                this.refresh_tool_info(cx);
            },
            cx,
        );
    }

    /// Open a session tab that immediately shows the raw spawn error as a
    /// failed run (rule 9), and release the in-flight key this run took.
    fn spawn_error_session(
        &mut self,
        spec: String,
        kind: &str,
        e: CoreError,
        cx: &mut Context<Self>,
    ) {
        let session = self.state.terminal.begin(format!("{kind} {spec}"));
        self.state
            .terminal
            .push_event_to(session, StreamEvent::Stderr(e.to_string()));
        self.state
            .terminal
            .push_event_to(session, StreamEvent::Exited(-1));
        self.state.finish_run(kind, &spec);
        cx.notify();
    }

    /// Bridge a core streaming receiver onto a background thread + async
    /// channel and route every event into the run's OWN terminal session;
    /// the caller's `done` hook runs once when the run exits.
    fn stream_to_terminal(
        &mut self,
        session: usize,
        rx: Receiver<StreamEvent>,
        done: impl Fn(&mut RootView, &mut Context<RootView>) + 'static,
        cx: &mut Context<RootView>,
    ) {
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
                let done_event = matches!(event, StreamEvent::Exited(_));
                this.update(cx, |this, cx| {
                    this.state.terminal.push_event_to(session, event.clone());
                    if done_event {
                        done(this, cx);
                    }
                    cx.notify();
                })
                .ok();
                if done_event {
                    break;
                }
            }
        })
        .detach();
    }

    /// Start an install (install ONLY, never `mise use` — AGENTS.md rule 4)
    /// and stream its output into a new terminal session. The std receiver
    /// from core is bridged onto an async channel by a background thread —
    /// the UI thread only awaits, never touching the entity off-thread.
    pub(crate) fn run_install(&mut self, tool: &str, version: &str, cx: &mut Context<Self>) {
        // Capture the run's OWN session index: streams must be routed into
        // that session, never "the active one" (concurrent installs and
        // mid-run tab clicks otherwise corrupt each other's output).
        let (session, rx) = match self.state.start_install(tool, version) {
            Ok((session, rx)) => (session, rx),
            Err(e) => {
                // Surface the raw error inside its own terminal tab, as a
                // failed run (AGENTS.md rule 9), not a silent nothing.
                let session = self
                    .state
                    .terminal
                    .begin(format!("install {tool}@{version}"));
                self.state
                    .terminal
                    .push_event_to(session, StreamEvent::Stderr(e.to_string()));
                self.state
                    .terminal
                    .push_event_to(session, StreamEvent::Exited(-1));
                cx.notify();
                return;
            }
        };

        self.stream_to_terminal(
            session,
            rx,
            |this, _cx| {
                // Refresh only on completion: rule 4 — after the
                // install finished the panel can honestly show
                // 「已安装 · 未生效」 with the apply actions exposed.
                this.refresh_installed();
            },
            cx,
        );
    }

    /// Start `mise upgrade` (spec = None for 全部升级, Some(name) for a single
    /// row) and stream its output into a NEW terminal session, like
    /// `run_install`. When the run exits, both the outdated list and the
    /// installed list are refreshed so the page and the badge reflect the
    /// upgrade that just happened. `bump` is chosen by the upgrade page's
    /// toggle; the tool-detail upgrade action passes `false`.
    pub(crate) fn run_upgrade(&mut self, spec: Option<String>, bump: bool, cx: &mut Context<Self>) {
        if let Some(name) = spec.as_deref() {
            self.state.begin_run("upgrade", name);
        }
        let (session, rx) = match self.state.start_upgrade(spec.as_deref(), bump) {
            Ok((session, rx)) => (session, rx),
            Err(e) => {
                // Surface the raw error inside its own terminal tab, as a
                // failed run (AGENTS.md rule 9), not a silent nothing.
                // For a named spec this routes through `spawn_error_session`
                // so the `begin_run` key taken above is released — without
                // it the per-tool upgrade button would stay disabled
                // forever after one failed spawn.
                if let Some(name) = &spec {
                    self.spawn_error_session(name.clone(), "upgrade", e, cx);
                    return;
                }
                let session = self.state.terminal.begin("upgrade".to_string());
                self.state
                    .terminal
                    .push_event_to(session, StreamEvent::Stderr(e.to_string()));
                self.state
                    .terminal
                    .push_event_to(session, StreamEvent::Exited(-1));
                cx.notify();
                return;
            }
        };

        let done_spec = spec.clone();
        self.stream_to_terminal(
            session,
            rx,
            move |this, cx| {
                if let Some(name) = &done_spec {
                    this.state.finish_run("upgrade", name);
                }
                // Refresh only on completion: the upgrade run changed
                // what is installed AND what counts as outdated, so
                // the page rows and the sidebar badge must both catch
                // up ("available upgrades" must not go stale).
                this.state.outdated = Loadable::Loading;
                this.refresh_outdated();
                this.refresh_installed();
                // An upgrade changed installed versions: re-enrich the
                // detail pane if one is open.
                this.refresh_tool_info(cx);
            },
            cx,
        );
    }

    /// Per-tool upgrade from the tool detail pane (spec §6.3): upgrades the
    /// upgraded tool only, scope follows the current selection, no `--bump`.
    pub(crate) fn run_tool_upgrade(&mut self, name: &str, cx: &mut Context<Self>) {
        self.run_upgrade(Some(name.to_string()), false, cx);
    }

    // -------------------------------------------------------------------
    // Environment page (M3-T2 / M3-T3)
    // -------------------------------------------------------------------

    /// Load the env page data for the CURRENT selection scope
    /// (declarations + effective) and, on a project page, ALSO the global
    /// declarations (the diff view's 全局 column). Blocking subprocess calls
    /// run on a background thread; the result is applied only while the
    /// selection's scope still matches, the same stale-guard pattern as
    /// `load_outdated_for_selection`. Failures land in `Loadable::Failed`
    /// with the raw message (rule 9) and feed the trust banner.
    ///
    /// `force = true` bypasses the coalescing predicate: a JUST-FINISHED
    /// `mise set`/`unset` leaves the scope and the Ready snapshot untouched
    /// while the DATA it lists changed, so a plain (unforced) reload would
    /// coalesce early and the new/deleted variable would stay invisible
    /// until the next page switch.
    pub(crate) fn load_env_for_selection(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(expected) = self.state.env_scope_key() else {
            return;
        };
        // Coalesce via the shared pure predicate (same semantics as for
        // `outdated`): reuse a Ready snapshot of the same scope, or an
        // in-flight fetch for the SAME scope — unless forced.
        if !AppState::should_load_env(
            self.state.env_in_flight.as_deref(),
            self.state.env_scope.as_deref(),
            matches!(self.state.env_declarations, Loadable::Ready(_))
                && matches!(self.state.env_effective, Loadable::Ready(_)),
            &expected,
            force,
        ) {
            return;
        }
        let Some(mise) = self.state.mise.clone() else {
            return;
        };

        self.state.env_in_flight = Some(expected.clone());
        // A slow listing must not be read as "empty": mark loading first,
        // same as the installed list does at startup.
        self.state.env_declarations = Loadable::Loading;
        self.state.env_effective = Loadable::Loading;
        // The other scope is loaded only for a project page (there the
        // ALL-GLOBAL declaration table fills the diff's 全局 column; on a
        // global page there is no single project to compare against and the
        // column stays neutral).
        let fetch_other_scope = self.state.selection.project_id().is_some();

        let root = self.state.current_project_root();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            let declarations = mise.env_declarations(root.as_deref());
            let effective = mise.env_extended(root.as_deref());
            // Global scope listing for `mise set` (root = None). Not `--global`:
            // `set` has no such output flag; the unscoped invocation lists the
            // Global-scope declarations.
            let other = if fetch_other_scope {
                Some(mise.env_declarations(None))
            } else {
                None
            };
            tx.send_blocking((declarations, effective, other)).ok();
        });
        cx.notify();

        let expected = expected.clone();
        cx.spawn(async move |this, cx| {
            if let Ok((declarations, effective, other)) = rx.recv().await {
                this.update(cx, |this, cx| {
                    if this.state.env_in_flight.as_deref() == Some(expected.as_str()) {
                        this.state.env_in_flight = None;
                    }
                    // Stale-result guard: apply only while the selection's
                    // scope is still the one this fetch was started for.
                    if this.state.env_scope_key().as_deref() == Some(expected.as_str()) {
                        this.state.env_declarations = match declarations {
                            Ok(rows) => Loadable::Ready(rows),
                            Err(e) => {
                                this.state.note_untrusted(&e);
                                Loadable::Failed(e.to_string())
                            }
                        };
                        this.state.env_effective = match effective {
                            Ok(rows) => Loadable::Ready(rows),
                            Err(e) => {
                                this.state.note_untrusted(&e);
                                Loadable::Failed(e.to_string())
                            }
                        };
                        this.state.env_other_declarations = match other {
                            None => Loadable::Idle,
                            Some(Ok(rows)) => Loadable::Ready(rows),
                            Some(Err(e)) => Loadable::Failed(e.to_string()),
                        };
                        this.state.env_scope = Some(expected);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Exact re-read (`mise set <NAME>`) for an ambiguous declaration row:
    /// its parsed value may be off, so the display replaces it with the
    /// exact value once this async Read finishes.
    pub(crate) fn load_env_exact(&mut self, name: String, cx: &mut Context<Self>) {
        // Coalesce: a Loading/Ready exact value is not refetched on repeat.
        if self
            .state
            .env_exact
            .get(&name)
            .is_some_and(|s| matches!(s, Loadable::Loading | Loadable::Ready(_)))
        {
            return;
        }
        let Some(mise) = self.state.mise.clone() else {
            return;
        };
        let root = self.state.current_project_root();
        self.state.env_exact.insert(name.clone(), Loadable::Loading);
        cx.notify();

        let (tx, rx) = async_channel::unbounded();
        let thread_name = name.clone();
        std::thread::spawn(move || {
            let result = mise.env_value(&thread_name, root.as_deref());
            tx.send_blocking(result).ok();
        });
        let expected_scope = self.state.env_scope_key();
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                this.update(cx, |this, cx| {
                    // Apply only while the fetch's scope is still the current
                    // one (an exact value of another scope must not bleed).
                    if this.state.env_scope_key() == expected_scope {
                        this.state.env_exact.insert(
                            name,
                            match result {
                                Ok(v) => Loadable::Ready(v),
                                Err(e) => Loadable::Failed(e.to_string()),
                            },
                        );
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Reveal / re-hide one row's value. Revealing an AMBIGUOUS declaration
    /// (its parsed value may be off) ALSO starts the exact `mise set <NAME>`
    /// re-read so the display becomes exact — planned M3-T2 behavior.
    pub(crate) fn toggle_env_reveal(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.env_revealed.remove(name) {
            self.env_revealed.insert(name.to_string());
            // Exact read for ambiguous declarations: locate the current
            // scope's declaration for the name.
            let ambiguous = self
                .state
                .env_declarations
                .ready()
                .and_then(|rows| rows.iter().find(|d| d.name == name))
                .is_some_and(|d| d.ambiguous);
            if ambiguous {
                self.load_env_exact(name.to_string(), cx);
            }
        }
        cx.notify();
    }

    /// Expand / collapse one declaration row's override chain.
    pub(crate) fn toggle_env_expanded(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.env_expanded.remove(name) {
            self.env_expanded.insert(name.to_string());
        }
        cx.notify();
    }

    /// Copy a value to the clipboard — ONLY when the user can see it
    /// (per-row reveal or the 显示全部 toggle, `env::copy_allowed`); a hidden
    /// value refuses with a notice instead of silently landing on the
    /// clipboard (sensitive-value handling).
    pub(crate) fn copy_env_value(
        &mut self,
        name: &str,
        raw: &str,
        show_all: bool,
        cx: &mut Context<Self>,
    ) {
        // mise's `[redacted]` placeholder is never a real value (states
        // truthfully rather than planting the literal text in the clipboard).
        if mise_core::is_redacted(raw) {
            self.notice = Some(t!("env.copy_redacted_warn").to_string());
            cx.notify();
            return;
        }
        if !env::copy_allowed(show_all, &self.env_revealed, name) {
            self.notice = Some(t!("env.copy_hidden_warn").to_string());
            cx.notify();
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(raw.to_string()));
        self.notice = None;
        cx.notify();
    }

    /// Open the 「新增 / 编辑变量」 dialog. The input entities are created
    /// HERE (before the dialog opens — dialog rebuilds must not recreate
    /// them); editing pre-fills the value input from the current scope's
    /// declaration. NOTHING is written by this function; the write is
    /// `run_set_env` in the dialog's OK handler (rule 3 + rule 10).
    pub(crate) fn prompt_env_edit(
        &mut self,
        editing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prefilled: Option<String> = editing.clone().and_then(|name| {
            let decl = self
                .state
                .env_declarations
                .ready()
                .and_then(|rows| rows.iter().find(|d| d.name == name))?;
            // The parsed value is only a safe starting point when the row's
            // layout was recovered with certainty and the value is not mise's
            // `[redacted]` placeholder; ambiguous/redacted rows use the exact
            // `mise set NAME` re-read, or refuse the edit when it has not
            // landed (confirming the parsed value would write corrupted
            // text back — Fix C of the whole-branch review).
            let exact = self.state.env_exact.get(&name).and_then(|s| match s {
                Loadable::Ready(v) => Some(v.as_str()),
                _ => None,
            });
            match env::edit_prefill(exact, decl) {
                env::Prefill::Declared(value) | env::Prefill::Exact(value) => Some(value),
                env::Prefill::Refuse => None,
            }
        });
        // Refusal closes the requested edit with a clear notice — AND starts
        // the exact read so a second click (once it resolves) succeeds.
        let refuses_edit = editing.is_some()
            && prefilled.is_none()
            && self.state.env_declarations.ready().is_some_and(|rows| {
                editing
                    .as_deref()
                    .is_some_and(|n| rows.iter().any(|d| d.name == n))
            });
        if refuses_edit {
            let name = editing.clone().unwrap();
            self.notice = Some(t!("env.edit_refuse_ambiguous").to_string());
            self.load_env_exact(name, cx);
            cx.notify();
            return;
        }
        // The edited declaration's own file: the write targets THIS file
        // (`mise set --file`), not the page's default scope file.
        let editing_source: Option<String> = editing.as_deref().and_then(|name| {
            self.state.env_declarations.ready().and_then(|rows| {
                rows.iter()
                    .find(|d| d.name == name)
                    .map(|d| d.source.display().to_string())
            })
        });
        let has_project = self.state.current_project_root().is_some();

        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("env.name").to_string()));
        let prefill = prefilled.clone();
        let value_input = cx.new(|cx| {
            let state = InputState::new(window, cx);
            match &prefill {
                Some(value) => state.default_value(value.clone()),
                None => state,
            }
        });

        let name_sub = cx.subscribe(&name_input, |_this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let value_sub = cx.subscribe(&value_input, |_this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });

        // Default scope follows the page: project page → project config,
        // global page → global config ("write where you are"). Editing with
        // a known source ignores the scope (the declaration's file decides).
        let scope_default_global =
            !has_project || editing.is_none() && self.state.selection.project_id().is_none();
        self.env_dialog = Some(EnvDialogState {
            name_input: name_input.clone(),
            value_input: value_input.clone(),
            editing: editing.clone(),
            editing_source: editing_source.clone(),
            scope_global: scope_default_global,
        });

        let weak = cx.weak_entity();
        let title_key = if editing.is_some() {
            "env.edit_title"
        } else {
            "env.add_title"
        };
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title(t!(title_key).to_string())
                .child(env::edit_dialog_body(weak.clone(), cx))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("env.edit_ok").to_string())
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true)
                        .cancel_text(t!("nav.cancel").to_string())
                        .on_ok({
                            let weak = weak.clone();
                            move |_, _, cx| {
                                // Keep the dialog open (return false) while the
                                // name is invalid — the inline error already shows
                                // why; on a valid name the dialog closes and the
                                // write runs (nothing else here writes anything).
                                if let Some(view) = weak.upgrade() {
                                    view.update(cx, |this, cx| {
                                        let Some(dlg) = this.env_dialog.as_ref() else {
                                            return true;
                                        };
                                        let typed_name =
                                            dlg.name_input.read(cx).value().to_string();
                                        let typed_value =
                                            dlg.value_input.read(cx).value().to_string();
                                        let scope_global = dlg.scope_global;
                                        let file = dlg.editing_source.clone();
                                        // The WRITTEN name is the RESOLVED one
                                        // (editing fixes the name, adding uses
                                        // the typed text) — validated, so an
                                        // empty/invalid `NAME=VALUE` pair can
                                        // never reach `mise set`.
                                        let resolved_name =
                                            resolved_env_name(dlg.editing.as_deref(), &typed_name);
                                        if !env::valid_env_name(&resolved_name) {
                                            cx.notify();
                                            return false;
                                        }
                                        this.env_dialog = None;
                                        this.run_set_env(
                                            &resolved_name,
                                            &typed_value,
                                            file,
                                            scope_global,
                                            cx,
                                        );
                                        true
                                    })
                                } else {
                                    true
                                }
                            }
                        }),
                )
        });

        self._env_dialog_subscriptions = vec![name_sub, value_sub];
        cx.notify();
    }

    /// Stream `mise set NAME=VALUE` after the dialog confirmed the preview.
    /// With `file` set, the write targets that exact declaration file via
    /// `mise set --file` (editing an existing declaration); otherwise the
    /// scope toggle decides (`--global` / `-C`). Output streams to the
    /// terminal panel; on Exited the env page data is reloaded FORCED — the
    /// write changed the declarations while scope + Ready snapshot did not.
    pub(crate) fn run_set_env(
        &mut self,
        name: &str,
        value: &str,
        file: Option<String>,
        global: bool,
        cx: &mut Context<Self>,
    ) {
        self.state.begin_run("env-set", name);
        let result = match file.as_deref() {
            Some(file) => self.state.start_set_env_file(name, value, Path::new(file)),
            None => self.state.start_set_env(name, value, global),
        };
        let (session, rx) = match result {
            Ok(pair) => pair,
            Err(e) => {
                self.spawn_error_session(format!("{name}={value}"), "set", e, cx);
                return;
            }
        };
        let done_name = name.to_string();
        self.stream_to_terminal(
            session,
            rx,
            move |this, cx| {
                this.state.finish_run("env-set", &done_name);
                this.load_env_for_selection(true, cx);
            },
            cx,
        );
    }

    /// Ask for confirmation, then stream `mise unset`.
    ///
    /// The target is the row's OWN source file when one is known
    /// (`unset_target`: `mise [-C dir] unset --file <file> NAME`) — a global
    /// row on a project page otherwise runs `mise -C proj unset NAME`, which
    /// exits 0 while removing NOTHING (a silent no-op). Without a source the
    /// page's scope is targeted instead, and the confirmation SAYS so (rule
    /// 10: impact shown first; destructive actions are never hidden).
    pub(crate) fn prompt_env_delete(
        &mut self,
        name: String,
        source: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page_global = self.state.selection.project_id().is_none();
        let target = env::unset_target(source.as_deref(), page_global);
        let body = match &target {
            env::UnsetTarget::File(file) => t!(
                "env.delete_confirm_body",
                name = name.clone(),
                file = file.clone()
            )
            .to_string(),
            env::UnsetTarget::Scope { global } => t!(
                "env.delete_confirm_scope",
                name = name.clone(),
                scope = if *global {
                    t!("env.scope_global").to_string()
                } else {
                    t!("env.scope_project").to_string()
                }
            )
            .to_string(),
        };
        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            // The dialog builder may run again on rebuild; clone per build.
            let name = name.clone();
            let target = target.clone();
            let body = body.clone();
            let weak = weak.clone();
            alert
                .confirm()
                .title(t!("env.delete_confirm_title").to_string())
                .description(body)
                .ok_text(t!("env.delete").to_string())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(t!("nav.cancel").to_string())
                .on_ok(move |_, _, cx| {
                    if let Some(view) = weak.upgrade() {
                        view.update(cx, |this, cx| this.run_unset_env(&name, target.clone(), cx));
                    }
                    true
                })
        });
    }

    /// Internal helper the delete prompt uses to actually run the unset:
    /// `--file` against the declaration's own file, or the page-scope form
    /// when no source was known. On Exited the env page data reloads FORCED.
    pub(crate) fn run_unset_env(
        &mut self,
        name: &str,
        target: env::UnsetTarget,
        cx: &mut Context<Self>,
    ) {
        self.state.begin_run("env-unset", name);
        let result = match &target {
            env::UnsetTarget::File(file) => self.state.start_unset_env_file(name, Path::new(file)),
            env::UnsetTarget::Scope { global } => self.state.start_unset_env(name, *global),
        };
        let (session, rx) = match result {
            Ok(pair) => pair,
            Err(e) => {
                self.spawn_error_session(name.to_string(), "unset", e, cx);
                return;
            }
        };
        let done_name = name.to_string();
        self.stream_to_terminal(
            session,
            rx,
            move |this, cx| {
                this.state.finish_run("env-unset", &done_name);
                this.load_env_for_selection(true, cx);
            },
            cx,
        );
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
        // The 「应用版本」 flow on the discover page: busy state for the
        // exact spec the banner would apply, and whether a project is in
        // context at all (without one an apply labeled "this project" would
        // silently be global — mislabeled scopes are dishonest, rule-wise).
        let apply_spec = not_active_version
            .as_ref()
            .zip(self.state.versions_tool.as_deref())
            .map(|(version, tool)| format!("{tool}@{version}"));
        let apply_busy = apply_spec
            .as_deref()
            .is_some_and(|spec| self.state.is_busy("use", spec));
        discover::render(
            cx,
            &self.discover_input,
            &self.discover_hits,
            &installed_names,
            &self.discover_scroll,
            &self.state.registry,
            &query,
            &self.state.versions,
            self.state.versions_tool.as_deref(),
            not_active_version,
            self.state.current_project_root().is_some(),
            apply_busy,
            &self.state.deep_hits,
        )
    }

    /// Retry the registry load after a failure (discover page): `ensure_registry`
    /// re-runs `mise registry --json` because the state is `Failed`, and renders
    /// the raw error again if it still fails (AGENTS.md rule 9).
    pub(crate) fn retry_registry(&mut self, cx: &mut Context<Self>) {
        self.state.registry = Loadable::Loading;
        if let Err(e) = self.state.ensure_registry() {
            self.state.note_untrusted(&e);
        }
        cx.notify();
    }

    /// User-triggered deep search (`mise search --all`, online). Runs only on
    /// the explicit 「深度搜索」 click — never per keystroke. The blocking
    /// call runs on a background thread; the UI thread only awaits the
    /// bridged channel (same pattern as `select_discover_tool`), so the
    /// window keeps rendering while the request is in flight.
    pub(crate) fn run_deep_search(&mut self, cx: &mut Context<Self>) {
        let query = self.discover_input.read(cx).value().trim().to_string();
        // Blank query never touches the core (online nonsense request).
        if !AppState::deep_search_query_runs(&query) {
            return;
        }
        let Some(mise) = self.state.mise.clone() else {
            self.state.deep_hits =
                Loadable::Failed(t!("discover.versions_unavailable").to_string());
            cx.notify();
            return;
        };
        self.state.deep_query = Some(query.clone());
        self.state.deep_hits = Loadable::Loading;
        cx.notify();

        let root = self.state.current_project_root();
        let (tx, rx) = async_channel::unbounded();
        let thread_query = query.clone();
        std::thread::spawn(move || {
            let result = mise.deep_search(&thread_query, root.as_deref());
            tx.send_blocking(result).ok();
        });
        let expected_query = query;
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                this.update(cx, |this, cx| {
                    // Stale-result guard: a newer query superseded this fetch.
                    if this.state.deep_query.as_deref() == Some(expected_query.as_str()) {
                        this.state.deep_hits = match result {
                            Ok(hits) => Loadable::Ready(hits),
                            Err(e) => {
                                this.state.note_untrusted(&e);
                                Loadable::Failed(e.to_string())
                            }
                        };
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Retry the last deep search (its query is preserved) when it failed.
    pub(crate) fn retry_deep_search(&mut self, cx: &mut Context<Self>) {
        if self
            .state
            .deep_query
            .as_deref()
            .is_some_and(AppState::deep_search_query_runs)
        {
            self.run_deep_search(cx);
        }
    }

    /// Explicit `mise trust <path>` for the banner path. The CLI call is
    /// quick (no streaming) and runs synchronously like the startup loads;
    /// the trust result refreshes what the untrusted error had blocked.
    pub(crate) fn run_trust(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.state.trust_prompt.clone() else {
            return;
        };
        let Some(mise) = self.state.mise.clone() else {
            return;
        };
        match mise.trust(&path) {
            Ok(()) => {
                // Trust applied: drop the banner and re-run the loads an
                // untrusted config had failed.
                self.state.trust_prompt = None;
                self.state.installed = Loadable::Loading;
                self.refresh_installed();
                self.load_outdated_for_selection(cx);
                if matches!(self.state.registry, Loadable::Failed(_)) {
                    self.state.registry = Loadable::Loading;
                    let _ = self.state.ensure_registry();
                }
                cx.notify();
            }
            Err(e) => {
                self.notice = Some(e.to_string());
                cx.notify();
            }
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

    fn upgrade_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        upgrade::render(cx, &self.state.outdated, self.upgrade_bump)
    }

    /// Settings page (M5-T2): snapshot everything the builder needs up
    /// front — the view builders must never `cx.entity().read(cx)`
    /// mid-render (the entity is leased out then; a second read panics).
    fn settings_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = settings::SettingsSnapshot {
            theme_names: theme::theme_names(cx),
            current_theme: self.settings.theme.clone(),
            current_locale: self.settings.locale.clone(),
            diagnostics: self.state.diagnostics.clone(),
            diagnostics_in_flight: self.state.diagnostics_in_flight,
            update: self.state.update.clone(),
            update_in_flight: self.state.update_in_flight,
            url_draft: self.update_url_input.read(cx).value().to_string(),
            settings_path: self.settings_path.clone(),
            store_path: self.state.store_path.clone(),
        };
        settings::render(cx, snapshot, self.update_url_input.clone())
    }

    /// Environment page: snapshot the view-local state once and hand it to
    /// `env::render` (no re-entrant reads mid-layout).
    fn env_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter_text = self.env_filter_input.read(cx).value().to_string();
        // Snapshots read off `self.state` up front: the view builders must
        // never call `cx.entity().read(cx)` during render (the view entity
        // is leased out then — a second read panics).
        let project_in_context = self.state.selection.project_id().is_some();
        let project_root = self
            .state
            .current_project_root()
            .map(|p| p.to_string_lossy().into_owned());
        env::render(
            cx,
            self.env_tab,
            self.env_show_all,
            &self.env_filter_input,
            &self.state.env_effective,
            &self.state.env_declarations,
            &self.state.env_other_declarations,
            &self.state.env_exact,
            &self.env_revealed,
            &self.env_expanded,
            &filter_text,
            project_in_context,
            project_root,
        )
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
        // Idle counts as loading too: it only ever happens between the
        // selection change and the (synchronous) refresh below it — never as
        // a settled "no tools installed".
        let loading = !matches!(
            self.state.installed,
            Loadable::Ready(_) | Loadable::Failed(_)
        );
        let list = tool_list::render(
            cx,
            &self.search_input,
            rows,
            selected_key.as_deref(),
            filter,
            show_filter,
            loading,
            !search_query.is_empty(),
            load_error,
        );
        // Per-tool actions (apply / uninstall / upgrade) read the CURRENT
        // rows and the outdated list; spec for the busy keys is the
        // `tool@version` the dialog targets.
        let detail = match &selected_row {
            Some(row) => {
                let spec = format!("{}@{}", row.name, row.version);
                // Scope-gated: the tool must be outdated in the CURRENT
                // scope — a stale global snapshot must never offer an
                // upgrade that would run project-scope (or vice versa).
                let can_upgrade = self
                    .state
                    .outdated_from_current_scope()
                    .is_some_and(|list| list.iter().any(|t| t.name == row.name));
                // Snapshot BEFORE the view builder: apply scope resolution
                // must not read the view entity inside `tool_detail::render`.
                let apply_global = self.state.current_project_root().is_none();
                tool_detail::render(
                    cx,
                    row,
                    &self.state.tool_info,
                    self.state.tool_info_tool.as_deref(),
                    can_upgrade,
                    self.state.is_busy("use", &spec),
                    self.state.is_busy("uninstall", &spec),
                    self.state.is_busy("upgrade", &row.name),
                    apply_global,
                )
                .into_any_element()
            }
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

/// Persistent untrusted-config banner: names the config path and offers the
/// explicit 「信任此配置」 action. Rendered AT THE ROOT (above every page)
/// and driven by `state.trust_prompt`, so it survives page switches until
/// the user trusts or quit — an untrusted config breaks every mise call,
/// not one page's.
fn trust_banner(
    cx: &Context<RootView>,
    path: &std::path::Path,
    on_trust: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .w_full()
        .h_flex()
        .gap_2()
        .px_3()
        .py_2()
        .text_sm()
        .text_color(theme.warning)
        .bg(theme.warning.opacity(0.12))
        .child(t!("trust.banner", path = path.display().to_string()).to_string())
        .child(
            Button::new("trust-config")
                .outline()
                .xsmall()
                .label(t!("trust.button").to_string())
                .on_click(on_trust),
        )
}

impl Render for RootView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.mise.is_none() {
            let on_detect = cx.listener(|this, _event, _window, cx| {
                this.state.detect();
                // mise may have appeared only now (setup page path): the
                // startup load was a no-op back then. Failures stay in
                // `state.registry` and surface on the discover page.
                if let Err(e) = this.state.ensure_registry() {
                    this.state.note_untrusted(&e);
                }
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

        let right_pane = match &self.state.selection {
            Selection::GlobalTools | Selection::ProjectTools(_) => {
                self.tools_pane(cx).into_any_element()
            }
            Selection::Discover => self.discover_pane(cx).into_any_element(),
            Selection::Upgrade => self.upgrade_pane(cx).into_any_element(),
            Selection::GlobalEnv | Selection::ProjectEnv(_) => self.env_pane(cx).into_any_element(),
            Selection::GlobalTasks | Selection::ProjectTasks(_) => {
                self.tasks_pane(cx).into_any_element()
            }
            Selection::Settings => self.settings_pane(cx).into_any_element(),
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
            .when_some(self.state.trust_prompt.clone(), |d, path| {
                let on_trust = cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.run_trust(cx);
                });
                d.child(trust_banner(cx, &path, on_trust))
            })
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
            // Custom title bar: the native traffic lights stay (see
            // `main.rs`), the app title is gone, and the language / theme
            // pickers live on this row instead of a separate header.
            .child(titlebar::render(
                cx,
                &theme_names,
                &current_theme,
                &current_locale,
            ))
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
                        self.state.outdated_count(),
                    ))
                    .child(right_col),
            )
            .when(has_sessions, |root| {
                root.child(terminal::render(&self.state.terminal, cx))
            })
            .into_any_element()
    }
}
