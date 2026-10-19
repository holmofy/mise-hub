use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStore {
    pub projects: Vec<Project>,
    pub last_selected: Option<String>,
}

impl ProjectStore {
    pub fn add(&mut self, path: &Path) -> Result<&Project, CoreError> {
        let canonical = path.canonicalize()?;
        let id = canonical.to_string_lossy().into_owned();
        if let Some(idx) = self.projects.iter().position(|p| p.id == id) {
            return Ok(&self.projects[idx]);
        }
        let name = canonical
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        self.projects.push(Project { id, path: canonical, name });
        Ok(self.projects.last().expect("just pushed"))
    }

    pub fn remove(&mut self, id: &str) {
        self.projects.retain(|p| p.id != id);
        if self.last_selected.as_deref() == Some(id) {
            self.last_selected = None;
        }
    }

    pub fn load(path: &Path) -> Result<Self, CoreError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

pub fn default_store_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "mise-hub", "mise-hub")
        .map(|d| d.config_dir().join("projects.json"))
        .unwrap_or_else(|| PathBuf::from("projects.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_dedupes_by_canonical_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::default();
        store.add(dir.path()).unwrap();
        store.add(dir.path()).unwrap();
        assert_eq!(store.projects.len(), 1);
        assert_eq!(store.projects[0].name, dir.path().file_name().unwrap().to_string_lossy().to_string());
    }

    #[test]
    fn missing_file_loads_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::load(&dir.path().join("none.json")).unwrap();
        assert!(store.projects.is_empty());
    }

    #[test]
    fn roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.json");
        let mut store = ProjectStore::default();
        store.add(dir.path()).unwrap();
        store.save(&path).unwrap();
        let loaded = ProjectStore::load(&path).unwrap();
        assert_eq!(loaded.projects, store.projects);
    }

    #[test]
    fn remove_drops_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::default();
        let id = store.add(dir.path()).unwrap().id.clone();
        store.remove(&id);
        assert!(store.projects.is_empty());
    }
}
