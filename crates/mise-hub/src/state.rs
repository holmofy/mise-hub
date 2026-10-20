use mise_core::{CoreError, Mise, ProjectStore, ScopeFilter, ToolEntry, ToolRow, filter_rows};
use mise_core::{RemoteVersion, SearchHit, SearchIndex, StreamEvent};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Discover,
    GlobalTools,
    GlobalTasks,
    GlobalEnv,
    ProjectTools(String),
    ProjectTasks(String),
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
    /// Async-filled by the view through `queue_remote_versions`; a failure
    /// keeps the raw error in `Loadable::Failed` for the panel to render.
    pub versions: Loadable<Vec<RemoteVersion>>,
    /// Which tool `versions` belongs to; guards against a stale result
    /// arriving after the user picked another tool.
    pub versions_tool: Option<String>,
    /// Bottom terminal panel sessions.
    pub terminal: TerminalState,
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

    /// Start `mise install tool@version` (install ONLY — never `mise use`,
    /// AGENTS.md rule 4), open a new terminal session for it, and hand the
    /// event receiver to the caller; the VIEW consumes it on the UI thread.
    /// Does not touch any UI state beyond the terminal session.
    pub fn start_install(
        &mut self,
        tool: &str,
        version: &str,
    ) -> Result<std::sync::mpsc::Receiver<StreamEvent>, mise_core::CoreError> {
        let Some(mise) = &self.mise else {
            return Err(mise_core::CoreError::NotFound);
        };
        let root = self.current_project_root();
        let rx = mise.install_stream(tool, version, root.as_deref())?;
        self.terminal.begin(format!("install {tool}@{version}"));
        Ok(rx)
    }

    /// The install state of a tool as the UI should present it, per AGENTS.md
    /// rule 4: installing and activating are separate steps. `Some(version)`
    /// means the tool is on the machine but NOT active in the current
    /// project (discover installs always go global — no `mise.toml` is ever
    /// written from here).
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
    /// Unique id (returned by `TerminalState::begin`); not used by the UI
    /// yet — the tab bar addresses sessions by index — but kept as the
    /// stable handle for future `terminal.close(id)`.
    #[allow(dead_code)]
    pub id: String,
    pub title: String,
    pub lines: Vec<TerminalLine>,
    pub status: RunStatus,
}

impl TerminalSession {
    /// Append raw output text to this session, shaped like real process
    /// output: the streamer emits one logical line per event (its `lines()`
    /// already strips the trailing `\n`), ANSI escape sequences are stripped
    /// so raw codes never reach the user (red line), and a line containing
    /// carriage returns is a progress update — its final fragment REPLACES
    /// the previous line instead of appending a new one.
    fn feed(&mut self, stream: Stream, text: &str) {
        let clean = strip_ansi(text);
        // Allow multi-line payloads: everything before a `\n` is complete.
        for (ix, piece) in clean.split('\n').enumerate() {
            let is_last = ix == clean.split('\n').count() - 1;
            if is_last && piece.is_empty() && clean.ends_with('\n') {
                break; // trailing newline: no empty line
            }
            if piece.contains('\r') {
                // Progress bar: the last fragment is the visible state.
                let visible = piece.rsplit('\r').next().unwrap_or("");
                match self.lines.last_mut() {
                    Some(line) => line.text = visible.to_string(),
                    None => self.lines.push(TerminalLine {
                        stream,
                        text: visible.to_string(),
                    }),
                }
            } else {
                self.lines.push(TerminalLine {
                    stream,
                    text: piece.to_string(),
                });
            }
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
    /// Open a new run, focus it, and make the panel visible. Ids are unique
    /// (tabs are addressed by index; titles may repeat).
    pub fn begin(&mut self, title: impl Into<String>) -> String {
        let id = format!("term-{}", self.sessions.len() + 1);
        self.sessions.push(TerminalSession {
            id: id.clone(),
            title: title.into(),
            lines: vec![],
            status: RunStatus::Running,
        });
        self.active = self.sessions.len() - 1;
        self.visible = true;
        id
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

    pub fn push_event(&mut self, event: StreamEvent) {
        let Some(session) = self.sessions.get_mut(self.active) else {
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

        state.start_install("ripgrep", "14.1.0").unwrap();

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
        term.push_event(StreamEvent::Stdout("downloading".into()));
        term.push_event(StreamEvent::Stderr("warning".into()));
        term.push_event(StreamEvent::Exited(0));
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
        term.push_event(StreamEvent::Exited(1));
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
        term.push_event(StreamEvent::Stdout("one".into()));
        let second = term.begin("install b@2");
        term.push_event(StreamEvent::Stdout("two".into()));

        assert_ne!(first, second);
        assert!(term.visible);
        assert_eq!(term.sessions.len(), 2);
        let session = term.active_session().unwrap();
        assert_eq!(session.id, second);
        assert_eq!(session.title, "install b@2");
        assert_eq!(session.lines.len(), 1);
        assert_eq!(session.lines[0].text, "two");
    }

    /// AGENTS.md rule 9: events may arrive before any session exists
    /// (spawn failure racing the panel) — never panic.
    #[test]
    fn push_event_without_session_is_noop() {
        let mut term = TerminalState::default();
        term.push_event(StreamEvent::Stdout("orphan".into()));
        term.push_event(StreamEvent::Exited(0));
        assert!(term.active_session().is_none());
    }

    /// Program output arrives '\n'-terminated per event from the streamer.
    #[test]
    fn trailing_newline_does_not_create_empty_line() {
        let mut term = TerminalState::default();
        term.begin("install x");
        term.push_event(StreamEvent::Stdout("line one\nline two\n".into()));
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

    /// A carriage-return progress update rewrites the tail instead of
    /// appending a new line.
    #[test]
    fn carriage_return_rewrites_partial_line() {
        let mut term = TerminalState::default();
        term.begin("install x");
        term.push_event(StreamEvent::Stdout("downloading  10%".into()));
        term.push_event(StreamEvent::Stdout("downloading  50%\rdone!".into()));
        let session = term.active_session().unwrap();
        assert_eq!(session.lines.len(), 1);
        assert_eq!(session.lines[0].text, "done!");
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
