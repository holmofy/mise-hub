use crate::error::CoreError;
use crate::types::InstalledTool;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFilter {
    All,
    Project,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRow {
    pub name: String,
    pub version: String,
    pub requested_version: Option<String>,
    pub scope: Scope,
    pub active: bool,
    pub install_path: Option<PathBuf>,
    pub source_path: Option<PathBuf>,
}

pub struct InstalledSet {
    tools: BTreeMap<String, Vec<InstalledTool>>,
}

impl InstalledSet {
    pub fn parse(json: &str) -> Result<Self, CoreError> {
        Ok(Self {
            tools: serde_json::from_str(json)?,
        })
    }

    pub fn rows(&self, project_root: Option<&Path>) -> Vec<ToolRow> {
        let mut rows = Vec::new();
        for (name, versions) in &self.tools {
            for tool in versions {
                let path = tool.source.as_ref().and_then(|s| s.path.as_deref());
                let scope = match (project_root, path) {
                    (Some(root), Some(p)) if p.starts_with(root) => Scope::Project,
                    _ => Scope::Global,
                };
                rows.push(ToolRow {
                    name: name.clone(),
                    version: tool.version.clone(),
                    requested_version: tool.requested_version.clone(),
                    scope,
                    active: tool.active,
                    install_path: tool.install_path.clone(),
                    source_path: path.map(Path::to_path_buf),
                });
            }
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
        rows
    }
}

pub fn filter_rows(rows: &[ToolRow], filter: ScopeFilter) -> Vec<ToolRow> {
    rows.iter()
        .filter(|r| match filter {
            ScopeFilter::All => true,
            ScopeFilter::Project => r.scope == Scope::Project,
            ScopeFilter::Global => r.scope == Scope::Global,
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SAMPLE: &str = r#"{
      "node": [
        {"version":"20.11.0","requested_version":"20","install_path":"/p/node/20.11.0",
         "source":{"type":"mise.toml","path":"/proj/mise.toml"},"installed":true,"active":true},
        {"version":"22.9.0","requested_version":"22","install_path":"/p/node/22.9.0",
         "source":{"type":"mise.toml","path":"/home/u/.config/mise/config.toml"},"installed":true,"active":false}
      ],
      "ripgrep": [
        {"version":"14.1.0","install_path":"/p/rg/14.1.0","installed":true,"active":true}
      ]
    }"#;

    #[test]
    fn classifies_by_project_root() {
        let set = InstalledSet::parse(SAMPLE).unwrap();
        let rows = set.rows(Some(Path::new("/proj")));
        assert!(
            rows.iter()
                .any(|r| r.name == "node" && r.version == "20.11.0" && r.scope == Scope::Project)
        );
        assert!(
            rows.iter()
                .any(|r| r.name == "node" && r.version == "22.9.0" && r.scope == Scope::Global)
        );
    }

    #[test]
    fn missing_source_defaults_to_global() {
        let set = InstalledSet::parse(SAMPLE).unwrap();
        let rows = set.rows(Some(Path::new("/proj")));
        let rg = rows.iter().find(|r| r.name == "ripgrep").unwrap();
        assert_eq!(rg.scope, Scope::Global);
    }

    #[test]
    fn null_or_pathless_source_defaults_to_global() {
        let json = r#"{"go":[
          {"version":"1.22.0","source":null,"installed":true,"active":false},
          {"version":"1.23.0","source":{"type":"mise.toml"},"installed":true,"active":false}
        ]}"#;
        let rows = InstalledSet::parse(json)
            .unwrap()
            .rows(Some(Path::new("/proj")));
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.scope == Scope::Global && r.source_path.is_none())
        );
        assert!(
            rows.iter()
                .all(|r| r.requested_version.is_none() && r.install_path.is_none())
        );
    }

    #[test]
    fn filters_by_scope() {
        let rows = InstalledSet::parse(SAMPLE)
            .unwrap()
            .rows(Some(Path::new("/proj")));
        assert_eq!(filter_rows(&rows, ScopeFilter::Project).len(), 1);
        assert_eq!(filter_rows(&rows, ScopeFilter::Global).len(), 2);
        assert_eq!(filter_rows(&rows, ScopeFilter::All).len(), 3);
    }
}
