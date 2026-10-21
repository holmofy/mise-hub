use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEntry {
    pub short: String,
    #[serde(default)]
    pub backends: Vec<String>,
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InstalledTool {
    pub version: String,
    #[serde(default)]
    pub requested_version: Option<String>,
    #[serde(default)]
    pub install_path: Option<std::path::PathBuf>,
    #[serde(default)]
    pub source: Option<ConfigSource>,
    #[serde(default = "default_true")]
    pub installed: bool,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ConfigSource {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub path: Option<std::path::PathBuf>,
}

fn default_true() -> bool {
    true
}

/// Payload of `mise tool <name> --json`.
///
/// Note: the plan sketch had `config_source: Option<String>`, but the real
/// mise output is an object `{ "type", "path" }`, so it is modeled as
/// `Option<ConfigSource>` (see report).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolInfo {
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub installed_versions: Vec<String>,
    #[serde(default)]
    pub requested_versions: Option<Vec<String>>,
    #[serde(default)]
    pub active_versions: Option<Vec<String>>,
    #[serde(default)]
    pub config_source: Option<ConfigSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteVersion {
    pub version: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_info_parses_full_payload() {
        // Real shape captured from `mise tool node --json` (mise 2026.10.15),
        // fields not modeled by ToolInfo are ignored by serde.
        let json = r#"{
            "backend": "core:node",
            "description": null,
            "url": null,
            "installed_versions": ["24.22.0"],
            "requested_versions": ["24.22.0"],
            "active_versions": ["24.22.0"],
            "config_source": {
                "type": "mise.toml",
                "path": "/Users/x/.config/mise/config.toml"
            },
            "tool_options": { "os": null, "depends": null },
            "security": [{ "type": "checksum", "algorithm": "sha256" }]
        }"#;
        let info: ToolInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.backend, "core:node");
        assert_eq!(info.installed_versions, vec!["24.22.0"]);
        assert_eq!(info.description, None);
        assert_eq!(info.config_source.unwrap().kind, "mise.toml");
    }

    #[test]
    fn tool_info_defaults_missing_fields() {
        let info: ToolInfo = serde_json::from_str(r#"{"backend":"core:node"}"#).unwrap();
        assert_eq!(info.backend, "core:node");
        assert!(info.installed_versions.is_empty());
        assert_eq!(info.description, None);
        assert_eq!(info.url, None);
        assert_eq!(info.requested_versions, None);
        assert_eq!(info.active_versions, None);
        assert_eq!(info.config_source, None);
    }
}
