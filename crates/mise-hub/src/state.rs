use mise_core::{CoreError, Mise, ProjectStore, ScopeFilter, ToolEntry, ToolRow, filter_rows};
use mise_core::{SearchHit, SearchIndex};
use std::path::{Path, PathBuf};

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
    pub mise: Option<Mise>,
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
        let mise = Mise::new(
            program,
            std::sync::Arc::new(mise_core::ProcessRunner),
            std::sync::Arc::new(mise_core::ProcessStreamer),
        );
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
        state.mise = Some(mise);

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
        state.mise = Some(mise);

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
        state.mise = Some(mise);
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
        state.mise = Some(mise);
        state.selection = Selection::GlobalTools;
        state.refresh_installed().unwrap();

        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["ls".to_string(), "--json".to_string()]
        );
    }
}
