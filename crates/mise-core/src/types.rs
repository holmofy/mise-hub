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
