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
