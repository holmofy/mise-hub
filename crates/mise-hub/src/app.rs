use crate::i18n;
use crate::state::{AppState, ApplyFlow, Loadable, Selection};
use crate::theme;
use crate::views::{discover, setup, sidebar, terminal, tool_detail, tool_list, upgrade};
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
use mise_core::{AppSettings, CoreError, ScopeFilter, SearchHit, StreamEvent, ToolRow};
use rust_i18n::t;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

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
    /// The 「包含超出配置范围的版本（--bump）」 toggle on the upgrade page.
    pub upgrade_bump: bool,
    /// One open 「应用版本」 flow: spec, scope, pin choice and the async
    /// `mise use --dry-run` preview the confirm dialog renders. See
    /// `ApplyFlow` — the dialog reads this on every rebuild, so the validator
    /// stays honest about what will be written before anything is written.
    pub apply_flow: Option<ApplyFlow>,
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
            upgrade_bump: false,
            apply_flow: None,
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
            Err(e) => self.state.installed = Loadable::Failed(e.to_string()),
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
            Err(e) => self.state.outdated = Loadable::Failed(e.to_string()),
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
                            Err(e) => Loadable::Failed(e.to_string()),
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
                                Err(e) => Loadable::Failed(e.to_string()),
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
        )
    }

    /// Retry the registry load after a failure (discover page): `ensure_registry`
    /// re-runs `mise registry --json` because the state is `Failed`, and renders
    /// the raw error again if it still fails (AGENTS.md rule 9).
    pub(crate) fn retry_registry(&mut self, cx: &mut Context<Self>) {
        self.state.registry = Loadable::Loading;
        let _ = self.state.ensure_registry();
        cx.notify();
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
                tool_detail::render(
                    cx,
                    row,
                    &self.state.tool_info,
                    self.state.tool_info_tool.as_deref(),
                    can_upgrade,
                    self.state.is_busy("use", &spec),
                    self.state.is_busy("uninstall", &spec),
                    self.state.is_busy("upgrade", &row.name),
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
        let search_input = self.search_input.clone();
        let discover_input = self.discover_input.clone();
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
                                    let search_input = search_input.clone();
                                    let discover_input = discover_input.clone();
                                    move |_, window, cx| {
                                        i18n::set_locale(code);
                                        if let Some(view) = weak.upgrade() {
                                            view.update(cx, |this, cx| {
                                                this.settings.locale = code.to_string();
                                                let _ = this.settings.save(&settings_path);
                                                cx.notify();
                                            });
                                        }
                                        // The search placeholders were baked into
                                        // InputState at startup; re-evaluate them
                                        // from the new locale so they follow the
                                        // language switch without a restart.
                                        search_input.update(cx, |input, cx| {
                                            input.set_placeholder(
                                                t!("tools.search_installed").to_string(),
                                                window,
                                                cx,
                                            )
                                        });
                                        discover_input.update(cx, |input, cx| {
                                            input.set_placeholder(
                                                t!("discover.search").to_string(),
                                                window,
                                                cx,
                                            )
                                        });
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
            Selection::Upgrade => self.upgrade_pane(cx).into_any_element(),
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
