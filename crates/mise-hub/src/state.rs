use mise_core::manage::UsePreview;
use mise_core::{
    CoreError, Mise, OutdatedTool, ProjectStore, ScopeFilter, ToolEntry, ToolRow, filter_rows,
};
use mise_core::{RemoteVersion, SearchHit, SearchIndex, StreamEvent, types::ToolInfo};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Discover,
    Upgrade,
    GlobalTools,
    GlobalTasks,
    GlobalEnv,
    ProjectTools(String),
    // Milestone scaffolding: Tasks is wired up in M4, Environment in M3.
    #[allow(dead_code)] // M4 scaffolding
    ProjectTasks(String),
    #[allow(dead_code)] // M3 scaffolding
    ProjectEnv(String),
}

impl Selection {
    pub fn project_id(&self) -> Option<&str> {
        match self {
            Selection::ProjectTools(id)
            | Selection::ProjectTasks(id)
            | Selection::ProjectEnv(id) => Some(id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loadable<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Loadable<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Loadable::Ready(v) => Some(v),
            _ => None,
        }
    }
}

pub struct AppState {
    pub mise: Option<Arc<Mise>>,
    pub mise_version: Option<String>,
    pub projects: ProjectStore,
    /// Where `add_project` / `remove_project` persist the store. Production
    /// uses the real user store; tests must point this at a tempdir-backed
    /// path so the developer's `projects.json` is never written.
    pub store_path: PathBuf,
    pub selection: Selection,
    pub registry: Loadable<Vec<ToolEntry>>,
    pub installed: Loadable<Vec<ToolRow>>,
    pub selected_tool: Option<String>,
    /// Local search index over the registry, built once when the registry
    /// loads so `search` never re-derives it per keystroke over the ~1067
    /// registry entries. `search` falls back to rebuilding from `registry`
    /// when this is absent (tests assign the registry directly).
    search_index: Option<SearchIndex>,
    /// Remote versions for the tool the user picked on the discover page.
    /// Filled asynchronously by the view (`select_discover_tool`); a failure
    /// keeps the raw error in `Loadable::Failed` for the panel to render.
    pub versions: Loadable<Vec<RemoteVersion>>,
    /// Which tool `versions` belongs to; guards against a stale result
    /// arriving after the user picked another tool.
    pub versions_tool: Option<String>,
    /// Bottom terminal panel sessions.
    pub terminal: TerminalState,
    /// Rows of `mise outdated` for the upgrade page, loaded with the current
    /// project root when a project page is selected (global scope otherwise).
    pub outdated: Loadable<Vec<OutdatedTool>>,
    /// The scope the `outdated` snapshot was taken in: `Some("global")` after
    /// a global listing, `Some(<project root>)` after a project-scope listing,
    /// `None` before any listing finished. Per-tool upgrade visibility must
    /// match this against the CURRENT scope, or a stale snapshot from one
    /// page would gate actions that run in another (scope consistency).
    pub outdated_scope: Option<String>,
    /// The scope key the IN-FLIGHT `outdated` fetch was started for (set when
    /// the fetch spawns, cleared when its result is applied). Tracked
    /// separately from `outdated_scope` so "a fetch for scope X is running"
    /// can never be confused with "the completed snapshot belongs to X" —
    /// a quick B → A → B page switch must not coalesce B's refetch with A's
    /// in-flight request (fix round 2 of the M2-T3 regression).
    pub outdated_in_flight: Option<String>,
    /// Tool detail from `mise tool <name> --json` for the selected tools-page
    /// row, loaded asynchronously (see `RootView::load_tool_info`); a failure
    /// keeps the raw error for the detail pane (rule 9) while basic fields
    /// from `mise ls` still render.
    pub tool_info: Loadable<ToolInfo>,
    /// Which tool `tool_info` belongs to; stale results are dropped.
    pub tool_info_tool: Option<String>,
    /// Specs of destructive/long runs currently in flight
    /// (`"uninstall:node@20"`, `"use:node@20"`, …), used to disable the
    /// matching action buttons while a run cannot be re-entered safely.
    pub in_flight: HashSet<String>,
}

impl AppState {
    pub fn new(projects: ProjectStore) -> Self {
        Self {
            mise: None,
            mise_version: None,
            projects,
            store_path: mise_core::default_store_path(),
            selection: Selection::Discover,
            registry: Loadable::Idle,
            installed: Loadable::Idle,
            selected_tool: None,
            search_index: None,
            versions: Loadable::Idle,
            versions_tool: None,
            terminal: TerminalState::default(),
            outdated: Loadable::Idle,
            outdated_scope: None,
            outdated_in_flight: None,
            tool_info: Loadable::Idle,
            tool_info_tool: None,
            in_flight: HashSet::new(),
        }
    }

    /// Override where the project store is persisted. `RootView` passes the
    /// real user store path it already resolved; tests must pass a
    /// `tempfile::tempdir()`-backed path to stay hermetic.
    pub fn with_store_path(mut self, path: PathBuf) -> Self {
        self.store_path = path;
        self
    }

    pub fn attach_mise(&mut self, program: std::path::PathBuf) -> bool {
        if !program.is_file() {
            return false;
        }
        let mise = Arc::new(Mise::new(
            program,
            std::sync::Arc::new(mise_core::ProcessRunner),
            std::sync::Arc::new(mise_core::ProcessStreamer),
        ));
        self.mise_version = mise.version().ok();
        self.mise = Some(mise);
        true
    }

    pub fn detect(&mut self) -> bool {
        match mise_core::locate_mise() {
            Some(path) => self.attach_mise(path),
            None => false,
        }
    }

    pub fn add_project(&mut self, path: &Path) -> Result<(), mise_core::CoreError> {
        self.projects.add(path)?;
        self.projects.save(&self.store_path)?;
        Ok(())
    }

    pub fn remove_project(&mut self, id: &str) {
        self.projects.remove(id);
        if self.selection.project_id() == Some(id) {
            self.selection = Selection::Discover;
        }
        let _ = self.projects.save(&self.store_path);
    }

    pub fn rows(&self, filter: ScopeFilter) -> Vec<ToolRow> {
        match &self.installed {
            Loadable::Ready(rows) => filter_rows(rows, filter),
            _ => vec![],
        }
    }

    pub fn current_project_root(&self) -> Option<std::path::PathBuf> {
        let id = self.selection.project_id()?;
        self.projects
            .projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.path.clone())
    }

    pub fn refresh_installed(&mut self) -> Result<(), mise_core::CoreError> {
        let Some(mise) = &self.mise else {
            return Ok(());
        };
        let root = self.current_project_root();
        let set = mise.installed(root.as_deref())?;
        self.installed = Loadable::Ready(set.rows(root.as_deref()));
        Ok(())
    }

    /// Number of outdated tools for the sidebar badge; zero while unknown so
    /// the badge stays hidden until the upgrade page actually loaded data.
    pub fn outdated_count(&self) -> usize {
        self.outdated.ready().map(|v| v.len()).unwrap_or(0)
    }

    /// The scope key `outdated` / `outdated_from_current_scope` are matched
    /// against: `Some(<project root>)` on a project page, `Some("global")`
    /// everywhere else (Selection::Upgrade carries no project id, so the
    /// upgrade listing and the upgrade run are both global — rule 2).
    pub fn outdated_scope_key(&self) -> Option<String> {
        match self.current_project_root() {
            Some(root) => Some(root.to_string_lossy().into_owned()),
            None => Some("global".to_string()),
        }
    }

    /// Pure coalesce predicate for the async `outdated` fetch. A new fetch
    /// must start EXCEPT when one is already in flight for this exact scope
    /// (`in_flight`, tracked separately from the completed snapshot's scope)
    /// or when a `Ready` snapshot for the same scope can be reused. This
    /// decision-point pure helper exists so the B → A → B regression
    /// (fix round 2) is testable without GPUI: with A's fetch in flight and
    /// B completed, the caller must refetch B instead of coalescing with
    /// A's request.
    pub fn should_load_outdated(
        in_flight: Option<&str>,
        completed_scope: Option<&str>,
        completed_is_ready: bool,
        expected: &str,
    ) -> bool {
        if in_flight == Some(expected) {
            return false;
        }
        completed_scope != Some(expected) || !completed_is_ready
    }

    /// The `outdated` snapshot ONLY when it was taken in the CURRENT scope:
    /// per-tool upgrade visibility builds on this, so a stale global
    /// snapshot can never gate a project-scope upgrade (or vice versa).
    pub fn outdated_from_current_scope(&self) -> Option<&[OutdatedTool]> {
        if self.outdated_scope != self.outdated_scope_key() {
            return None;
        }
        self.outdated.ready().map(|v| v.as_slice())
    }

    /// Load `mise outdated --json` for the upgrade page: with the current
    /// project root when a project page is selected (project scope), global
    /// scope otherwise (no `-C`, per AGENTS.md rule 2). The scope of THIS
    /// listing is always the scope `start_upgrade` upgrades in, closing the
    /// "list global, upgrade project" hole. Failures land in
    /// `Loadable::Failed` with the raw message (rule 9).
    pub fn refresh_outdated(&mut self) -> Result<(), CoreError> {
        let Some(mise) = &self.mise else {
            return Ok(());
        };
        let root = self.current_project_root();
        match mise.outdated(root.is_none(), root.as_deref()) {
            Ok(tools) => {
                self.outdated = Loadable::Ready(tools);
                // Tag the snapshot with its scope so per-tool upgrade
                // visibility can never be driven by another page's stale
                // listing (rule consistency: list and run share the scope).
                self.outdated_scope = self.outdated_scope_key();
                // The synchronous refresh owns the fetch end-to-end; any
                // coalesced in-flight marker is obsolete once it lands.
                self.outdated_in_flight = None;
                Ok(())
            }
            Err(e) => {
                self.outdated = Loadable::Failed(e.to_string());
                Err(e)
            }
        }
    }

    /// Start `mise upgrade` and open a new terminal session for it, exactly
    /// like `start_install`: the caller routes the receiver's events into the
    /// returned session index via `push_event_to`. Scope follows the current
    /// selection: a project page upgrades that project (`-C`, no `--global`),
    /// everything else (Upgrade page, global tools) is global-scope
    /// (`--global`, no `-C`). `spec == None` upgrades everything.
    pub fn start_upgrade(
        &mut self,
        spec: Option<&str>,
        bump: bool,
    ) -> Result<(usize, std::sync::mpsc::Receiver<StreamEvent>), mise_core::CoreError> {
        let Some(mise) = &self.mise else {
            return Err(mise_core::CoreError::NotFound);
        };
        let root = self.current_project_root();
        let global = root.is_none();
        let title = match spec {
            Some(spec) => format!("upgrade {spec}"),
            None => "upgrade".to_string(),
        };
        let rx = mise.upgrade_stream(spec, bump, global, root.as_deref())?;
        let session = self.terminal.begin(title);
        Ok((session, rx))
    }

    /// Install the tool@{version} spec: install ONLY — never `mise use` (rule 4)
    pub fn start_install(
        &mut self,
        tool: &str,
        version: &str,
    ) -> Result<(usize, std::sync::mpsc::Receiver<StreamEvent>), mise_core::CoreError> {
        let Some(mise) = &self.mise else {
            return Err(mise_core::CoreError::NotFound);
        };
        let root = self.current_project_root();
        let rx = mise.install_stream(tool, version, root.as_deref())?;
        let session = self.terminal.begin(format!("install {tool}@{version}"));
        Ok((session, rx))
    }

    /// Scope for a `mise use` action (use_dry_run / use_apply_stream):
    /// project page → project scope; everything else → global scope. This is
    /// the scope resolution pinned by the apply tests and mirrors which tool
    /// listing produced the version ("apply to where you see it").
    pub fn use_scope(&self, global: bool) -> (bool, Option<PathBuf>) {
        let root = self.current_project_root();
        (global || root.is_none(), if global { None } else { root })
    }

    /// The blocking `mise use --dry-run` call for one scope — it runs ON the
    /// app's background thread (never the UI thread). The scope comes from
    /// `use_scope` beforehand; this helper only wraps the core call so the
    /// preview path has a single named seam (testable with a fake runner).
    pub fn use_preview(
        mise: &Mise,
        spec: &str,
        global: bool,
        root: Option<&Path>,
    ) -> Result<UsePreview, CoreError> {
        mise.use_dry_run(spec, global, root)
    }

    /// Stream `mise use [--pin] <spec>` and open its terminal session. The
    /// caller registers/unregisters the run key via `begin_run`/`finish_run`
    /// so the apply button cannot be re-entered while it streams.
    pub fn start_use(
        &mut self,
        spec: &str,
        global: bool,
        pin: bool,
    ) -> Result<(usize, std::sync::mpsc::Receiver<StreamEvent>), mise_core::CoreError> {
        let Some(mise) = &self.mise else {
            return Err(mise_core::CoreError::NotFound);
        };
        let (global, root) = self.use_scope(global);
        let rx = mise.use_apply_stream(spec, global, pin, root.as_deref())?;
        let scope = if global { " (--global)" } else { "" };
        let session = self.terminal.begin(format!("use {spec}{scope}"));
        Ok((session, rx))
    }

    /// Stream `mise uninstall <tool>@<version>` with the current project root
    /// for `-C` context, and open its terminal session. Only removes the
    /// installed version — the config entry is NEVER touched (rule 3: writes
    /// go through mise commands; `uninstall` is machine-side, `use`/`set` is
    /// config-side and out of this path entirely).
    pub fn start_uninstall(
        &mut self,
        tool: &str,
        version: &str,
    ) -> Result<(usize, std::sync::mpsc::Receiver<StreamEvent>), mise_core::CoreError> {
        let Some(mise) = &self.mise else {
            return Err(mise_core::CoreError::NotFound);
        };
        let spec = uninstall_spec(tool, version);
        let root = self.current_project_root();
        let rx = mise.uninstall_stream(&spec, root.as_deref())?;
        let session = self.terminal.begin(format!("uninstall {spec}"));
        Ok((session, rx))
    }

    /// Mark a run in flight (`begin_run` before streaming, `finish_run` on
    /// Exited) so matching action buttons can disable themselves.
    pub fn begin_run(&mut self, kind: &str, spec: &str) {
        self.in_flight.insert(run_key(kind, spec));
    }

    pub fn finish_run(&mut self, kind: &str, spec: &str) {
        self.in_flight.remove(&run_key(kind, spec));
    }

    pub fn is_busy(&self, kind: &str, spec: &str) -> bool {
        self.in_flight.contains(&run_key(kind, spec))
    }

    /// The install state of a tool as the UI should present it, per AGENTS.md
    /// rule 4: installing and activating are separate steps. `Some(version)`
    /// means the tool is on the machine but NOT active in the current
    /// project (install never writes config — activation happens only via
    /// `mise use`, which is out of scope until M2).
    pub fn installed_but_not_active(&self, tool: &str) -> Option<String> {
        let rows = self.installed.ready()?;
        let matching: Vec<&mise_core::ToolRow> = rows.iter().filter(|r| r.name == tool).collect();
        let active_in_project = matching
            .iter()
            .any(|r| r.scope == mise_core::Scope::Project && r.active);
        if !active_in_project {
            matching.first().map(|r| r.version.clone())
        } else {
            None
        }
    }

    /// Ensure the tool registry is loaded exactly once.
    ///
    /// Runs `mise registry --json` through `mise-core` (blocking call; the
    /// caller owns the async/background decision the way `refresh_installed`
    /// does), marking the registry `Loading` first so the discover page can
    /// distinguish progress from an empty result. A second call after
    /// success is a no-op; after a failure it may retry, and the raw error
    /// message stays in `Loadable::Failed` for the UI to render verbatim
    /// (AGENTS.md rule 9).
    pub fn ensure_registry(&mut self) -> Result<(), CoreError> {
        if matches!(self.registry, Loadable::Ready(_)) {
            return Ok(());
        }
        let Some(mise) = &self.mise else {
            // mise not detected yet (setup page): nothing to load.
            return Ok(());
        };
        self.registry = Loadable::Loading;
        match mise.registry() {
            Ok(entries) => {
                // Build the index once, up front, so keystroke searches never
                // re-derive it over the ~1067 registry entries.
                self.search_index = Some(SearchIndex::new(entries.clone()));
                self.registry = Loadable::Ready(entries);
                Ok(())
            }
            Err(e) => {
                self.registry = Loadable::Failed(e.to_string());
                Err(e)
            }
        }
    }

    /// Search or browse the loaded registry. An empty query returns every
    /// entry alphabetically (browse mode); otherwise hits are ranked by the
    /// local index. Returns an empty vec while the registry is not loaded.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        if let Some(index) = &self.search_index {
            return index.search(query, limit);
        }
        // Fallback for registries assigned directly (tests): rebuild a
        // throwaway index. In production `search_index` is always present
        // once `registry` is Ready.
        match &self.registry {
            Loadable::Ready(entries) => SearchIndex::new(entries.clone()).search(query, limit),
            _ => vec![],
        }
    }
}

/// The spec `mise uninstall` receives: `{tool}@{version}`, passed verbatim
/// (backend prefixes like `npm:prettier` are never split or rewritten).
pub fn uninstall_spec(tool: &str, version: &str) -> String {
    format!("{tool}@{version}")
}

/// The in-flight bookkeeping key for a run kind + spec.
pub fn run_key(kind: &str, spec: &str) -> String {
    format!("{kind}:{spec}")
}

/// State of one open 「应用版本」 flow (tool detail pane / discover versions
/// pane): which spec, which scope the confirm dialog targets, the pin
/// choice made inside the dialog, and the `mise use --dry-run` preview the
/// dialog renders (Loading while the blocking call runs on a background
/// thread; Failed keeps the raw error, AGENTS.md rule 9). Nothing here ever
/// writes a config file — the write happens only in `run_use` after the
/// user confirms in the dialog (rule 4).
#[derive(Debug, Clone)]
pub struct ApplyFlow {
    pub spec: String,
    pub global: bool,
    pub pin: bool,
    pub preview: Loadable<UsePreview>,
}

/// Which locale key presents the post-apply activation status of a tool, as
/// a pure function of the refreshed `mise ls` rows: the tool is 已生效
/// (`tools.active`) when some row of the CURRENT scope lists it `active`,
/// and 已安装 · 未生效 (`discover.installed_not_active`) otherwise.
pub fn apply_status_key(active: bool) -> &'static str {
    if active {
        "tools.active"
    } else {
        "discover.installed_not_active"
    }
}

/// Which output stream a terminal line came from; drives its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One rendered line in a terminal session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLine {
    pub stream: Stream,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed(i32),
}

#[derive(Debug, Clone)]
pub struct TerminalSession {
    pub title: String,
    pub lines: Vec<TerminalLine>,
    pub status: RunStatus,
}

/// Maximum rendered lines kept per terminal session. A chatty install must
/// not grow the UI's memory without bound; only the newest lines are kept.
pub const MAX_TERMINAL_LINES: usize = 5000;

impl TerminalSession {
    /// Append raw output text to this session, shaped like real process
    /// output: the streamer emits one logical line per event (its `lines()`
    /// already strips the trailing `\n`), ANSI escape sequences are stripped
    /// so raw codes never reach the user (red line), and a `\r`-containing
    /// progress payload renders only its FINAL fragment — as a new line,
    /// never rewriting previously completed lines.
    fn feed(&mut self, stream: Stream, text: &str) {
        let clean = strip_ansi(text);
        // Compute the split count once: recomputing per iteration is O(n²).
        let line_count = clean.split('\n').count();
        // Allow multi-line payloads: everything before a `\n` is complete.
        for (ix, piece) in clean.split('\n').enumerate() {
            let is_last = ix == line_count - 1;
            if is_last && piece.is_empty() && clean.ends_with('\n') {
                break; // trailing newline: no empty line
            }
            // Progress bars arrive WITHIN one event (`"10%\r 50%\r100%"`):
            // the last fragment is the visible state and becomes its own
            // line. Earlier completed lines are never mutated.
            let visible = if piece.contains('\r') {
                piece.rsplit('\r').next().unwrap_or("")
            } else {
                piece
            };
            self.lines.push(TerminalLine {
                stream,
                text: visible.to_string(),
            });
        }
        // Cap retained output: keep only the newest `MAX_TERMINAL_LINES`
        // lines so a very chatty install cannot grow memory without bound.
        // Older lines are dropped from the front (fade like a real terminal).
        if self.lines.len() > MAX_TERMINAL_LINES {
            let excess = self.lines.len() - MAX_TERMINAL_LINES;
            self.lines.drain(0..excess);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TerminalState {
    pub sessions: Vec<TerminalSession>,
    pub active: usize,
    pub visible: bool,
}

impl TerminalState {
    /// Open a new run, focus it, make the panel visible, and return that
    /// session's INDEX — a stable handle for routing the run's events via
    /// `push_event_to`, immune to later tab clicks or concurrent runs.
    pub fn begin(&mut self, title: impl Into<String>) -> usize {
        self.sessions.push(TerminalSession {
            title: title.into(),
            lines: vec![],
            status: RunStatus::Running,
        });
        let index = self.sessions.len() - 1;
        self.active = index;
        self.visible = true;
        index
    }

    pub fn active_session(&self) -> Option<&TerminalSession> {
        self.sessions.get(self.active)
    }

    /// Focus a session by index; out-of-range indexes are ignored so a
    /// stale click can never panic.
    pub fn select(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.active = index;
        }
    }

    /// Drop the active session's rendered output, keeping the session (and
    /// its tab) itself.
    pub fn clear_active(&mut self) {
        if let Some(session) = self.sessions.get_mut(self.active) {
            session.lines.clear();
        }
    }

    /// Plain text of the active session (for clipboard / save).
    pub fn active_text(&self) -> Option<String> {
        self.active_session().map(|s| {
            s.lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// Route one stream event into the session at `index` — NOT the active
    /// one. Two runs may be streaming concurrently and the user may click
    /// tabs while they run; mis-routing would put one run's output in
    /// another run's tab. Out-of-range indexes are ignored (never panic).
    pub fn push_event_to(&mut self, index: usize, event: StreamEvent) {
        let Some(session) = self.sessions.get_mut(index) else {
            return;
        };
        match event {
            StreamEvent::Stdout(text) => session.feed(Stream::Stdout, &text),
            StreamEvent::Stderr(text) => session.feed(Stream::Stderr, &text),
            StreamEvent::Exited(0) => session.status = RunStatus::Succeeded,
            StreamEvent::Exited(code) => session.status = RunStatus::Failed(code),
        }
    }
}

/// Remove ANSI escape sequences (CSI and OSC forms) from process output.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            None => break,
            // CSI: parameters/intermediates up to a final byte 0x40–0x7E.
            Some('[') => {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if ('\x40'..='\x7e').contains(&n) {
                        break;
                    }
                }
            }
            // OSC: string terminated by BEL or ESC \ (ST).
            Some(']') => {
                chars.next();
                let mut prev_esc = false;
                for t in chars.by_ref() {
                    if t == '\x07' || (prev_esc && t == '\\') {
                        break;
                    }
                    prev_esc = t == '\x1b';
                }
            }
            // Single-character escape: consume the next char.
            Some(_) => {
                chars.next();
            }
        }
    }
    out
}

#[cfg(test)]
mod install_tests {
    use super::*;
    use mise_core::runner::{StreamEvent, StreamingRunner};
    use mise_core::{CommandOutput, CommandRequest, CommandRunner};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};
    struct RecordingRunner;
    impl CommandRunner for RecordingRunner {
        fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
            Ok(CommandOutput {
                code: 0,
                stdout: "{}".into(),
                stderr: String::new(),
            })
        }
    }

    /// Records the exact argv of every streamed `mise` invocation so the
    /// UI-layer install contract (`install <tool>@<version>`) is pinned.
    struct RecordingStreamer {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
    }
    impl StreamingRunner for RecordingStreamer {
        fn spawn(&self, req: CommandRequest) -> Result<Receiver<StreamEvent>, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(std::sync::mpsc::channel().1)
        }
    }

    fn recording_mise() -> (Mise, Arc<Mutex<Vec<Vec<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (
            Mise::new(
                "mise".into(),
                Arc::new(RecordingRunner),
                Arc::new(RecordingStreamer { seen: seen.clone() }),
            ),
            seen,
        )
    }

    #[test]
    fn start_install_pins_install_argv_for_current_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let root = projects.add(dir.path()).unwrap().id.clone();
        let (mise, seen) = recording_mise();
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::ProjectTools(root.clone());

        let (session, _rx) = state.start_install("ripgrep", "14.1.0").unwrap();
        assert_eq!(session, 0);

        // AGENTS.md rule 2: `-C <dir>` comes first, then the subcommand.
        assert_eq!(
            seen.lock().unwrap()[0],
            vec![
                "-C".to_string(),
                root,
                "install".to_string(),
                "ripgrep@14.1.0".to_string(),
            ]
        );
        // The session opened for this run flows into the terminal panel.
        assert!(state.terminal.visible);
        assert_eq!(state.terminal.sessions.len(), 1);
        assert_eq!(state.terminal.sessions[0].title, "install ripgrep@14.1.0");
        // Install never activates: state keeps the version so the UI can
        // render 「已安装 · 未生效」 (rule 4).
        state.installed = Loadable::Ready(vec![mise_core::ToolRow {
            name: "ripgrep".into(),
            version: "14.1.0".into(),
            requested_version: None,
            scope: mise_core::Scope::Global,
            active: false,
            install_path: None,
            source_path: None,
        }]);
        assert_eq!(
            state.installed_but_not_active("ripgrep"),
            Some("14.1.0".into())
        );
    }
}

#[cfg(test)]
mod terminal_tests {
    use super::*;
    use mise_core::StreamEvent;

    #[test]
    fn appends_lines_in_order_and_sets_status() {
        let mut term = TerminalState::default();
        term.begin("install ripgrep@14.1.0");
        term.push_event_to(0, StreamEvent::Stdout("downloading".into()));
        term.push_event_to(0, StreamEvent::Stderr("warning".into()));
        term.push_event_to(0, StreamEvent::Exited(0));
        let session = term.active_session().unwrap();
        assert_eq!(session.lines.len(), 2);
        assert_eq!(session.lines[0].text, "downloading");
        assert_eq!(session.lines[0].stream, Stream::Stdout);
        assert_eq!(session.lines[1].text, "warning");
        assert_eq!(session.lines[1].stream, Stream::Stderr);
        assert!(matches!(session.status, RunStatus::Succeeded));
    }

    #[test]
    fn non_zero_exit_marks_failure() {
        let mut term = TerminalState::default();
        term.begin("install x");
        term.push_event_to(0, StreamEvent::Exited(1));
        assert!(matches!(
            term.active_session().unwrap().status,
            RunStatus::Failed(1)
        ));
    }

    /// `begin` must open a NEW session, focus it and show the panel again —
    /// a second install must not append into the first run's tab.
    #[test]
    fn begin_creates_new_active_session_and_shows_panel() {
        let mut term = TerminalState::default();
        assert!(!term.visible);
        let first = term.begin("install a@1");
        term.push_event_to(first, StreamEvent::Stdout("one".into()));
        let second = term.begin("install b@2");
        term.push_event_to(second, StreamEvent::Stdout("two".into()));

        assert_ne!(first, second);
        assert!(term.visible);
        assert_eq!(term.sessions.len(), 2);
        assert_eq!(second, term.sessions.len() - 1);
        let session = term.active_session().unwrap();
        assert_eq!(session.title, "install b@2");
        assert_eq!(session.lines.len(), 1);
        assert_eq!(session.lines[0].text, "two");
    }

    /// Events must be routed to the session they belong to (a Stable index),
    /// never to "the active one": with two sessions, output pushed to index
    /// 0 lands only in session 0, and session 1's status stays untouched
    /// (blocker: tab clicks / concurrent runs must not cross-contaminate).
    #[test]
    fn push_event_to_routes_only_to_target_session() {
        let mut term = TerminalState::default();
        let a = term.begin("install a@1");
        let b = term.begin("install b@2");
        assert_eq!(term.active, b);

        // Click tab `a` mid-run: `b`'s events must still go to `b` and
        // `a`'s status must survive `b`'s exit.
        term.push_event_to(b, StreamEvent::Stdout("b line".into()));
        term.push_event_to(b, StreamEvent::Exited(0));
        term.push_event_to(a, StreamEvent::Stderr("a warning".into()));

        let session_a = &term.sessions[a];
        let session_b = &term.sessions[b];
        assert_eq!(
            session_a
                .lines
                .iter()
                .map(|l| (l.stream, l.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(Stream::Stderr, "a warning")]
        );
        assert!(matches!(session_a.status, RunStatus::Running));
        assert!(matches!(session_b.status, RunStatus::Succeeded));
    }

    /// AGENTS.md rule 9: events may arrive before any session exists
    /// (spawn failure racing the panel) — never panic.
    #[test]
    fn push_event_without_session_is_noop() {
        let mut term = TerminalState::default();
        term.push_event_to(0, StreamEvent::Stdout("orphan".into()));
        term.push_event_to(0, StreamEvent::Exited(0));
        assert!(term.active_session().is_none());
    }

    /// A very chatty run must not grow memory without bound: the session
    /// keeps only the last `MAX_TERMINAL_LINES` lines.
    #[test]
    fn feed_caps_lines_at_max_terminal_lines() {
        let mut term = TerminalState::default();
        term.begin("install chatty");
        let total = MAX_TERMINAL_LINES + 500;
        for i in 0..total {
            term.push_event_to(0, StreamEvent::Stdout(format!("line {i}")));
        }
        let session = term.active_session().unwrap();
        assert_eq!(session.lines.len(), MAX_TERMINAL_LINES);
        // The OLDEST lines were dropped: the last line is the newest output.
        assert_eq!(
            session.lines[session.lines.len() - 1].text,
            format!("line {}", total - 1)
        );
        assert_eq!(
            session.lines[0].text,
            format!("line {}", total - MAX_TERMINAL_LINES)
        );
    }

    /// A run that emits fewer than the cap keeps every line (cap never
    /// trims a short session).
    #[test]
    fn feed_under_cap_keeps_all_lines() {
        let mut term = TerminalState::default();
        term.begin("install small");
        for i in 0..10 {
            term.push_event_to(0, StreamEvent::Stdout(format!("line {i}")));
        }
        assert_eq!(term.active_session().unwrap().lines.len(), 10);
    }

    /// Program output arrives '\n'-terminated per event from the streamer.
    #[test]
    fn trailing_newline_does_not_create_empty_line() {
        let mut term = TerminalState::default();
        term.begin("install x");
        term.push_event_to(0, StreamEvent::Stdout("line one\nline two\n".into()));
        let session = term.active_session().unwrap();
        assert_eq!(
            session
                .lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            vec!["line one", "line two"]
        );
    }

    /// A `\r` progress payload arrives WITHIN one event; only its FINAL
    /// fragment is rendered, as a new line — previously completed lines are
    /// never overwritten.
    #[test]
    fn carriage_render_appends_final_fragment_without_rewriting_history() {
        let mut term = TerminalState::default();
        term.begin("install x");
        term.push_event_to(0, StreamEvent::Stdout("downloading  10%".into()));
        term.push_event_to(0, StreamEvent::Stdout("downloading  50%\rdone!".into()));
        let session = term.active_session().unwrap();
        assert_eq!(
            session
                .lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            // The earlier completed line survives; the \r event contributes
            // one new line holding only its final fragment.
            vec!["downloading  10%", "done!"]
        );
        // Multi-fragment progress within a single event yields one line too.
        term.push_event_to(0, StreamEvent::Stdout(" 10%\r 50%\r100%".into()));
        let session = term.active_session().unwrap();
        assert_eq!(session.lines.len(), 3);
        assert_eq!(session.lines[2].text, "100%");
    }

    /// Red line: install output must never show raw escape codes.
    #[test]
    fn ansi_escape_sequences_are_stripped() {
        assert_eq!(strip_ansi("\x1b[1;32mgreen\x1b[0m"), "green");
        assert_eq!(strip_ansi("\x1b[2K\x1b[1Gprogress"), "progress");
        assert_eq!(strip_ansi("\x1b]0;title\x07plain"), "plain");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }
}

#[cfg(test)]
mod search_state_tests {
    use super::*;
    use mise_core::ToolEntry;

    fn entry(short: &str) -> ToolEntry {
        ToolEntry {
            short: short.into(),
            backends: vec![],
            bins: vec![],
            description: None,
            aliases: vec![],
        }
    }

    #[test]
    fn search_uses_loaded_registry() {
        let mut state = AppState::new(ProjectStore::default());
        state.registry = Loadable::Ready(vec![entry("node"), entry("ripgrep")]);
        assert_eq!(state.search("rip", 10)[0].tool.short, "ripgrep");
        assert!(state.search("nothing", 10).is_empty());
    }

    #[test]
    fn empty_registry_returns_empty() {
        let state = AppState::new(ProjectStore::default());
        assert!(state.search("node", 10).is_empty());
    }

    /// Browse mode: an empty query must return every registry entry (sorted
    /// alphabetically), not an empty page.
    #[test]
    fn empty_query_returns_all_entries_alphabetically() {
        let mut state = AppState::new(ProjectStore::default());
        state.registry = Loadable::Ready(vec![entry("zig"), entry("node"), entry("python")]);
        let names: Vec<String> = state
            .search("", usize::MAX)
            .into_iter()
            .map(|h| h.tool.short)
            .collect();
        assert_eq!(names, vec!["node", "python", "zig"]);
    }
}

#[cfg(test)]
mod registry_load_tests {
    use super::*;
    use mise_core::CoreError;
    use mise_core::runner::{StreamEvent, StreamingRunner};
    use mise_core::{CommandOutput, CommandRequest, CommandRunner};
    use std::sync::{Arc, Mutex};

    struct RecordingRunner {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
        stdout: String,
    }

    impl CommandRunner for RecordingRunner {
        fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(CommandOutput {
                code: 0,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    struct NoopStreamer;
    impl StreamingRunner for NoopStreamer {
        fn spawn(
            &self,
            _req: CommandRequest,
        ) -> Result<std::sync::mpsc::Receiver<StreamEvent>, CoreError> {
            Ok(std::sync::mpsc::channel().1)
        }
    }

    fn recording_mise(stdout: &str) -> (Mise, Arc<Mutex<Vec<Vec<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = Arc::new(RecordingRunner {
            seen: seen.clone(),
            stdout: stdout.into(),
        });
        (
            Mise::new("mise".into(), runner, Arc::new(NoopStreamer)),
            seen,
        )
    }

    #[test]
    fn ensure_registry_runs_registry_json_once_and_is_cached() {
        let (mise, seen) = recording_mise(r#"[{"short":"node"}]"#);
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));

        state.ensure_registry().unwrap();
        // Calling again after success must not re-run the subprocess.
        state.ensure_registry().unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        // Global call: no `-C`, `registry --json` as the subcommand.
        assert_eq!(seen[0], vec!["registry".to_string(), "--json".to_string()]);
        assert_eq!(state.registry.ready().unwrap().len(), 1);
    }

    /// AGENTS.md rule 9: a bad JSON / failed CLI must surface as an error
    /// state (`Loadable::Failed`), not panic, not silently become empty.
    #[test]
    fn failed_registry_load_leaves_failed_state() {
        let (mise, _seen) = recording_mise("this is not json");
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));

        assert!(state.ensure_registry().is_err());
        assert!(matches!(state.registry, Loadable::Failed(_)));
        assert!(state.search("node", 10).is_empty());
    }

    #[test]
    fn ensure_registry_without_mise_leaves_registry_idle() {
        let mut state = AppState::new(ProjectStore::default());
        state.ensure_registry().unwrap();
        assert_eq!(state.registry, Loadable::Idle);
    }
}

#[cfg(test)]
mod upgrade_tests {
    use super::*;
    use mise_core::runner::{StreamEvent, StreamingRunner};
    use mise_core::{CommandOutput, CommandRequest, CommandRunner};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};

    struct RecordingRunner {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
        stdout: String,
    }
    impl CommandRunner for RecordingRunner {
        fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(CommandOutput {
                code: 0,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    struct RecordingStreamer {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
    }
    impl StreamingRunner for RecordingStreamer {
        fn spawn(&self, req: CommandRequest) -> Result<Receiver<StreamEvent>, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(std::sync::mpsc::channel().1)
        }
    }

    fn outdated_json() -> String {
        r#"{"node":{"name":"node","requested":"20","current":"20.10.0","latest":"20.20.2","source":{"path":"/proj/mise.toml"}}}"#.into()
    }

    // Multi-line return type only to keep clippy's type-complexity lint off
    // the two recording buffers this fixture hands back to each test.
    #[allow(clippy::type_complexity)]
    fn recording_mise(
        stdout: String,
    ) -> (
        Mise,
        Arc<Mutex<Vec<Vec<String>>>>,
        Arc<Mutex<Vec<Vec<String>>>>,
    ) {
        let seen_runs = Arc::new(Mutex::new(Vec::new()));
        let seen_streams = Arc::new(Mutex::new(Vec::new()));
        (
            Mise::new(
                "mise".into(),
                Arc::new(RecordingRunner {
                    seen: seen_runs.clone(),
                    stdout,
                }),
                Arc::new(RecordingStreamer {
                    seen: seen_streams.clone(),
                }),
            ),
            seen_runs,
            seen_streams,
        )
    }

    // AGENTS.md rule 2: `outdated` (a -C subcommand) must carry the selected
    // project root in front of the subcommand.
    #[test]
    fn refresh_outdated_uses_project_root_for_project_selection() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();
        let (mise, seen, _streams) = recording_mise(outdated_json());
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::ProjectTools(id.clone());

        state.refresh_outdated().unwrap();

        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C".to_string(), id, "outdated".into(), "--json".into()]
        );
        let tools = state.outdated.ready().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "node");
    }

    #[test]
    fn refresh_outdated_omits_root_globally() {
        let (mise, seen, _streams) = recording_mise("{}".into());
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::GlobalTools;

        state.refresh_outdated().unwrap();

        // The global listing must carry `--global` so the run checks the same
        // scope that `start_upgrade` upgrades in (no `-C` anywhere — AGENTS.md
        // rule 2: global calls never carry the project dir).
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["outdated".to_string(), "--json".into(), "--global".into()]
        );
    }

    // AGENTS.md rule 9: bad JSON must land in `Loadable::Failed` with the raw
    // error, never panic and never silently become an empty list.
    #[test]
    fn refresh_outdated_failure_keeps_raw_error() {
        let (mise, _seen, _streams) = recording_mise("not json".into());
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));

        assert!(state.refresh_outdated().is_err());
        assert!(matches!(state.outdated, Loadable::Failed(_)));
        assert_eq!(state.outdated_count(), 0);
    }

    #[test]
    fn refresh_outdated_without_mise_leaves_idle() {
        let mut state = AppState::new(ProjectStore::default());
        state.refresh_outdated().unwrap();
        assert_eq!(state.outdated, Loadable::Idle);
    }

    /// The snapshot must be tagged with its scope: project listing → the
    /// project root, global listing → "global". Per-tool upgrade visibility
    /// matches the CURRENT scope against this tag, so a stale snapshot from
    /// one page can never gate an action that runs in another scope.
    #[test]
    fn refresh_outdated_tags_snapshot_with_scope() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();
        let root = projects.projects[0].path.to_string_lossy().into_owned();
        let (mise, _seen, _streams) = recording_mise(outdated_json());
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));

        state.selection = Selection::ProjectTools(id);
        state.refresh_outdated().unwrap();
        assert_eq!(state.outdated_scope, Some(root.clone()));

        state.selection = Selection::Upgrade;
        state.refresh_outdated().unwrap();
        assert_eq!(state.outdated_scope, Some("global".into()));
    }

    /// Scope-gated visibility: an outdated snapshot taken in one scope must
    /// be invisible on a page whose actions would run in another scope.
    #[test]
    fn outdated_from_current_scope_requires_matching_scope() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();
        let (mise, _seen, _streams) = recording_mise(outdated_json());
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));

        // Fresh session: scope never loaded → nothing exposed.
        state.selection = Selection::ProjectTools(id.clone());
        assert!(state.outdated_from_current_scope().is_none());

        // Load in the project scope → visible on the same project page.
        state.refresh_outdated().unwrap();
        let tools = state.outdated_from_current_scope().unwrap();
        assert_eq!(tools[0].name, "node");

        // Switch to the Upgrade page (global scope): the project snapshot
        // must NOT gate global-scope actions.
        state.selection = Selection::Upgrade;
        assert!(state.outdated_from_current_scope().is_none());
        assert!(state.outdated_scope != state.outdated_scope_key());
    }

    // The coalesce predicate for the async outdated fetch, incl. the
    // fix-round-2 regression: B completed, A's fetch in flight, user back
    // on B → must refetch B (predicate true), not coalesce with A's fetch.
    #[test]
    fn should_load_outdated_coalesce_cases() {
        // Regression B → A → B: B's snapshot scope is still B, but the
        // loadable is Loading for A's in-flight fetch → must refetch B.
        assert!(AppState::should_load_outdated(
            Some("A"),
            Some("B"),
            false,
            "B"
        ));
        // In-flight fetch for the SAME scope → coalesce.
        assert!(!AppState::should_load_outdated(
            Some("B"),
            Some("A"),
            false,
            "B"
        ));
        // Ready snapshot of the same scope, nothing in flight → reuse.
        assert!(!AppState::should_load_outdated(None, Some("B"), true, "B"));
        // Ready snapshot of a DIFFERENT scope → must load.
        assert!(AppState::should_load_outdated(None, Some("A"), true, "B"));
        // Failed/Loading snapshot of same scope, nothing in flight → load.
        assert!(AppState::should_load_outdated(None, Some("B"), false, "B"));
        // Nothing loaded at all → load.
        assert!(AppState::should_load_outdated(None, None, false, "B"));
    }

    // The Upgrade page never sits on a project selection (Selection::Upgrade
    // carries no project id), so the run is global-scope: `--global`, no `-C`.
    #[test]
    fn start_upgrade_pins_global_argv_and_opens_session() {
        let (mise, _seen, streams) = recording_mise("{}".into());
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::Upgrade;

        let (session, _rx) = state.start_upgrade(Some("node"), false).unwrap();
        assert_eq!(session, 0);
        assert_eq!(
            streams.lock().unwrap()[0],
            vec![
                "upgrade".to_string(),
                "--global".to_string(),
                "node".to_string()
            ]
        );
        assert!(state.terminal.visible);
        assert_eq!(state.terminal.sessions.len(), 1);
        assert_eq!(state.terminal.sessions[0].title, "upgrade node");
        assert!(matches!(
            state.terminal.sessions[0].status,
            RunStatus::Running
        ));
    }

    #[test]
    fn start_upgrade_all_pins_bump_and_no_spec() {
        let (mise, _seen, streams) = recording_mise("{}".into());
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::Upgrade;

        let (session, _rx) = state.start_upgrade(None, true).unwrap();
        assert_eq!(
            streams.lock().unwrap()[0],
            vec![
                "upgrade".to_string(),
                "--bump".to_string(),
                "--global".to_string(),
            ]
        );
        assert_eq!(state.terminal.sessions[session].title, "upgrade");
    }

    #[test]
    fn outdated_count_is_zero_unless_ready() {
        let mut state = AppState::new(ProjectStore::default());
        assert_eq!(state.outdated_count(), 0);
        state.outdated = Loadable::Ready(vec![
            OutdatedTool {
                name: "node".into(),
                requested: None,
                current: None,
                latest: Some("20.20.2".into()),
                source_path: None,
            },
            OutdatedTool {
                name: "python".into(),
                requested: None,
                current: None,
                latest: Some("3.13".into()),
                source_path: None,
            },
        ]);
        assert_eq!(state.outdated_count(), 2);
    }
}

#[cfg(test)]
mod apply_action_tests {
    use super::*;
    use mise_core::runner::{StreamEvent, StreamingRunner};
    use mise_core::{CommandOutput, CommandRequest, CommandRunner};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};

    struct RecordingRunner {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
        stdout: String,
    }
    impl CommandRunner for RecordingRunner {
        fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(CommandOutput {
                code: 0,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    struct RecordingStreamer {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
    }
    impl StreamingRunner for RecordingStreamer {
        fn spawn(&self, req: CommandRequest) -> Result<Receiver<StreamEvent>, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(std::sync::mpsc::channel().1)
        }
    }

    #[allow(clippy::type_complexity)]
    fn recording_mise() -> (
        Mise,
        Arc<Mutex<Vec<Vec<String>>>>,
        Arc<Mutex<Vec<Vec<String>>>>,
    ) {
        let seen_runs = Arc::new(Mutex::new(Vec::new()));
        let seen_streams = Arc::new(Mutex::new(Vec::new()));
        (
            Mise::new(
                "mise".into(),
                Arc::new(RecordingRunner {
                    seen: seen_runs.clone(),
                    stdout: String::new(),
                }),
                Arc::new(RecordingStreamer {
                    seen: seen_streams.clone(),
                }),
            ),
            seen_runs,
            seen_streams,
        )
    }

    /// The apply preview is a plain dry-run: the blocking `mise use --dry-run`
    /// must carry the PROJECT scope (`-C`, no `--global`) when invoked from a
    /// project page, exactly like the eventual `use` write would.
    #[test]
    fn use_preview_project_scope_passes_dash_c() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();
        let (mise, seen, _streams) = recording_mise();
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::ProjectTools(id.clone());

        let (global, root) = state.use_scope(false);
        AppState::use_preview(
            state.mise.as_ref().unwrap(),
            "node@20",
            global,
            root.as_deref(),
        )
        .unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec![
                "-C".to_string(),
                id,
                "use".to_string(),
                "--dry-run".to_string(),
                "node@20".to_string(),
            ]
        );
    }

    /// The apply preview in a project-less context (discover, upgrade, global
    /// tools) or with an explicit global request is a `--global` dry run with
    /// no `-C` (AGENTS.md rule 2).
    #[test]
    fn use_preview_global_scope_omits_dash_c() {
        let (mise, seen, _streams) = recording_mise();
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::GlobalTools;

        let (global, root) = state.use_scope(true);
        AppState::use_preview(
            state.mise.as_ref().unwrap(),
            "node@20",
            global,
            root.as_deref(),
        )
        .unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec![
                "use".to_string(),
                "--dry-run".to_string(),
                "--global".to_string(),
                "node@20".to_string(),
            ]
        );
    }

    /// The apply write keeps the same scope as the preview, and `--pin` is
    /// inserted after `use` (the pin choice from the confirm dialog).
    #[test]
    fn start_use_pins_argv_and_scope() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();
        let (mise, _seen, streams) = recording_mise();
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::ProjectTools(id.clone());

        let (session, _rx) = state.start_use("node@20", false, true).unwrap();
        assert_eq!(
            streams.lock().unwrap()[0],
            vec![
                "-C".to_string(),
                id,
                "use".to_string(),
                "--pin".to_string(),
                "node@20".to_string(),
            ]
        );
        assert_eq!(state.terminal.sessions[session].title, "use node@20");
        assert!(state.terminal.visible);
    }

    // AGENTS.md rule 2 + the spec passed verbatim: backend-prefixed names are
    // never split or rewritten, and the project root comes first.
    #[test]
    fn start_uninstall_pins_spec_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();
        let (mise, _seen, streams) = recording_mise();
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::ProjectTools(id.clone());

        let (session, _rx) = state.start_uninstall("npm:prettier", "3").unwrap();
        assert_eq!(
            streams.lock().unwrap()[0],
            vec![
                "-C".to_string(),
                id,
                "uninstall".to_string(),
                "npm:prettier@3".to_string(),
            ]
        );
        assert_eq!(
            state.terminal.sessions[session].title,
            "uninstall npm:prettier@3"
        );
    }

    /// A per-tool upgrade on a project page stays in THIS project's scope.
    #[test]
    fn start_upgrade_project_scope_with_spec() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();
        let (mise, _seen, streams) = recording_mise();
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::ProjectTools(id.clone());

        state.start_upgrade(Some("node"), false).unwrap();
        assert_eq!(
            streams.lock().unwrap()[0],
            vec![
                "-C".to_string(),
                id,
                "upgrade".to_string(),
                "node".to_string()
            ]
        );
    }

    /// Rule 4 copy keys: active rows read 已生效, everything else keeps
    /// 「已安装 · 未生效」 until a successful apply flips the refreshed row.
    #[test]
    fn apply_status_key_switches_on_active() {
        assert_eq!(apply_status_key(true), "tools.active");
        assert_eq!(apply_status_key(false), "discover.installed_not_active");
    }

    #[test]
    fn run_key_combines_kind_and_spec() {
        assert_eq!(run_key("use", "node@20"), "use:node@20");
        assert_eq!(
            run_key("uninstall", "npm:prettier@3"),
            "uninstall:npm:prettier@3"
        );
    }

    #[test]
    fn uninstall_spec_is_verbatim_tool_at_version() {
        assert_eq!(uninstall_spec("node", "20"), "node@20");
        assert_eq!(uninstall_spec("npm:prettier", "3.1"), "npm:prettier@3.1");
    }

    /// begin/finish/existence circle for the in-flight key, so a run cannot
    /// be re-entered while streaming and the buttons can disable themselves.
    #[test]
    fn begin_run_and_finish_run_drive_is_busy() {
        let mut state = AppState::new(ProjectStore::default());
        assert!(!state.is_busy("use", "node@20"));
        state.begin_run("use", "node@20");
        assert!(state.is_busy("use", "node@20"));
        assert!(!state.is_busy("use", "other@1"));
        state.finish_run("use", "node@20");
        assert!(!state.is_busy("use", "node@20"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_roundtrips_project_id() {
        let sel = Selection::ProjectTools("p1".into());
        assert_eq!(sel.project_id(), Some("p1"));
        assert_eq!(Selection::Discover.project_id(), None);
    }

    #[test]
    fn loadable_ready_holds_value() {
        let l: Loadable<u32> = Loadable::Ready(3);
        assert_eq!(l.ready(), Some(&3));
        assert_eq!(Loadable::<u32>::Idle.ready(), None);
    }
}

#[cfg(test)]
mod detect_tests {
    use super::*;

    #[test]
    fn detect_without_mise_leaves_state_empty() {
        let mut state = AppState::new(ProjectStore::default());
        // 用一个确定不存在的路径模拟未安装
        assert!(!state.attach_mise(std::path::PathBuf::from("/nonexistent/mise")));
        assert!(state.mise.is_none());
    }
}

#[cfg(test)]
mod rows_tests {
    use super::*;
    use mise_core::{Scope, ToolRow};

    fn row(name: &str, scope: Scope) -> ToolRow {
        ToolRow {
            name: name.into(),
            version: "1.0.0".into(),
            requested_version: None,
            scope,
            active: false,
            install_path: None,
            source_path: None,
        }
    }

    #[test]
    fn all_shows_same_tool_twice_when_both_scopes() {
        let mut state = AppState::new(ProjectStore::default());
        state.installed = Loadable::Ready(vec![
            row("node", Scope::Project),
            row("node", Scope::Global),
        ]);
        let rows = state.rows(ScopeFilter::All);
        assert_eq!(rows.len(), 2);
        assert_eq!(state.rows(ScopeFilter::Project).len(), 1);
        assert_eq!(state.rows(ScopeFilter::Global).len(), 1);
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;

    /// Runs `f`, then asserts the developer's real project store file is
    /// byte-identical to before. Every store-writing test must point
    /// `AppState::store_path` at a tempdir (see `isolated_state`); this guard
    /// fails loudly if a regression ever writes the real user store.
    fn assert_real_store_untouched(f: impl FnOnce()) {
        let real = mise_core::default_store_path();
        let before = std::fs::read(&real).ok();
        f();
        assert_eq!(
            std::fs::read(&real).ok(),
            before,
            "test wrote the real project store at {}",
            real.display()
        );
    }

    /// An `AppState` whose store persists into a private tempdir. Each test
    /// gets its own tempdir, so parallel tests never share a file.
    fn isolated_state(store: &tempfile::TempDir) -> AppState {
        AppState::new(ProjectStore::default()).with_store_path(store.path().join("projects.json"))
    }

    #[test]
    fn add_project_dedupes_same_path() {
        assert_real_store_untouched(|| {
            let store = tempfile::tempdir().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let mut state = isolated_state(&store);
            state.add_project(dir.path()).unwrap();
            state.add_project(dir.path()).unwrap();
            assert_eq!(state.projects.projects.len(), 1);
            // The store file lands in the tempdir, not the user's config dir.
            assert!(store.path().join("projects.json").exists());
            assert_eq!(
                state.projects.projects[0].name,
                dir.path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            );
        });
    }

    #[test]
    fn remove_selected_project_resets_selection_to_discover() {
        assert_real_store_untouched(|| {
            let store = tempfile::tempdir().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let mut state = isolated_state(&store);
            let id = state.projects.add(dir.path()).unwrap().id.clone();
            state.selection = Selection::ProjectTools(id.clone());

            state.remove_project(&id);
            assert!(state.projects.projects.is_empty());
            assert_eq!(state.selection, Selection::Discover);
        });
    }

    #[test]
    fn remove_other_project_keeps_selection() {
        assert_real_store_untouched(|| {
            let store = tempfile::tempdir().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let mut state = isolated_state(&store);
            let kept = state.projects.add(dir.path()).unwrap().id.clone();
            state.selection = Selection::ProjectTools(kept.clone());

            state.remove_project("some-other-id");
            assert_eq!(state.selection, Selection::ProjectTools(kept));
        });
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use mise_core::runner::{StreamEvent, StreamingRunner};
    use mise_core::{CommandOutput, CommandRequest, CommandRunner};
    use std::sync::{Arc, Mutex};

    struct RecordingRunner {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
        stdout: String,
    }

    impl CommandRunner for RecordingRunner {
        fn run(&self, req: &CommandRequest) -> Result<CommandOutput, mise_core::CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(CommandOutput {
                code: 0,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    struct NoopStreamer;
    impl StreamingRunner for NoopStreamer {
        fn spawn(
            &self,
            _req: CommandRequest,
        ) -> Result<std::sync::mpsc::Receiver<StreamEvent>, mise_core::CoreError> {
            Ok(std::sync::mpsc::channel().1)
        }
    }

    fn recording_mise(stdout: &str) -> (Mise, Arc<Mutex<Vec<Vec<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = Arc::new(RecordingRunner {
            seen: seen.clone(),
            stdout: stdout.into(),
        });
        (
            Mise::new("mise".into(), runner, Arc::new(NoopStreamer)),
            seen,
        )
    }

    #[test]
    fn refresh_installed_uses_project_root_for_project_selection() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = ProjectStore::default();
        let id = projects.add(dir.path()).unwrap().id.clone();

        let (mise, seen) = recording_mise("{}");
        let mut state = AppState::new(projects);
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::ProjectTools(id.clone());
        state.refresh_installed().unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(
            seen[0],
            vec!["-C".to_string(), id, "ls".into(), "--json".into()]
        );
    }

    #[test]
    fn refresh_installed_omits_project_root_for_global() {
        let (mise, seen) = recording_mise("{}");
        let mut state = AppState::new(ProjectStore::default());
        state.mise = Some(Arc::new(mise));
        state.selection = Selection::GlobalTools;
        state.refresh_installed().unwrap();

        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["ls".to_string(), "--json".to_string()]
        );
    }
}
