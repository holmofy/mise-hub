use mise_core::{Mise, ProjectStore, ToolEntry, ToolRow};

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
            Selection::ProjectTools(id) | Selection::ProjectTasks(id) | Selection::ProjectEnv(id) => {
                Some(id)
            }
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
            selection: Selection::Discover,
            registry: Loadable::Idle,
            installed: Loadable::Idle,
            selected_tool: None,
        }
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
