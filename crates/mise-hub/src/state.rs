use mise_core::{Mise, ProjectStore, ScopeFilter, ToolEntry, ToolRow, filter_rows};
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

#[derive(Debug, Clone)]
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

    /// Ensure the tool registry is loaded.
    ///
    /// M1 does not render the registry yet (that is Task 12's discover page), so
    /// this is a stub that leaves the registry idle. Startup still calls it so
    /// the wiring is in place for the discover page.
    pub fn ensure_registry(&mut self) {}
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
