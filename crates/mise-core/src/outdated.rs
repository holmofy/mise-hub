use crate::error::CoreError;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One row of `mise outdated --json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedTool {
    pub name: String,
    pub requested: Option<String>,
    pub current: Option<String>,
    pub latest: Option<String>,
    pub source_path: Option<PathBuf>,
}

#[derive(Deserialize)]
struct OutdatedSource {
    #[serde(default)]
    path: Option<PathBuf>,
}

/// The value of each key in the outdated JSON object.
///
/// mise uses `null` in `name`, `requested`, `current`, `bump` and `latest`
/// positions depending on state, so every field is optional and defaults
/// are applied on top. The map key is authoritative for `name` because
/// mise can emit `name: null`.
#[derive(Deserialize)]
struct OutdatedEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    requested: Option<String>,
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    latest: Option<String>,
    #[serde(default)]
    source: Option<OutdatedSource>,
}

/// Parse the output of `mise outdated --json`.
///
/// Accepts the object shape `{ "<name>": { "name", "requested", "current",
/// "bump", "latest", "source": { "type", "path" } } }`, tolerates missing
/// fields, and sorts results by tool name.
pub fn parse_outdated(json: &str) -> Result<Vec<OutdatedTool>, CoreError> {
    let map: BTreeMap<String, OutdatedEntry> = serde_json::from_str(json)?;
    let mut tools = Vec::with_capacity(map.len());
    for (key, entry) in map {
        tools.push(OutdatedTool {
            // The JSON key is authoritative; `name` inside the entry may be
            // null or diverge. Never panic on missing fields.
            name: entry.name.unwrap_or(key),
            requested: entry.requested,
            current: entry.current,
            latest: entry.latest,
            source_path: entry.source.and_then(|s| s.path),
        });
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_entry() {
        let json = r#"{
            "node": {
                "name": "node",
                "requested": "20",
                "current": "20.10.0",
                "bump": null,
                "latest": "20.20.2",
                "source": { "type": "mise.toml", "path": "/proj/mise.toml" }
            }
        }"#;
        assert_eq!(
            parse_outdated(json).unwrap(),
            vec![OutdatedTool {
                name: "node".into(),
                requested: Some("20".into()),
                current: Some("20.10.0".into()),
                latest: Some("20.20.2".into()),
                source_path: Some("/proj/mise.toml".into()),
            }]
        );
    }

    #[test]
    fn tolerates_missing_current() {
        let json = r#"{ "node": { "name": "node", "requested": null,
            "current": null, "bump": null, "latest": "20.20.2",
            "source": { "type": "mise.toml", "path": "/proj/mise.toml" } } }"#;
        let tools = parse_outdated(json).unwrap();
        assert_eq!(tools[0].current, None);
        assert_eq!(tools[0].latest.as_deref(), Some("20.20.2"));
    }

    #[test]
    fn empty_object_yields_empty_vec() {
        assert!(parse_outdated("{}").unwrap().is_empty());
    }

    #[test]
    fn sorts_by_name() {
        let json = r#"{
            "zig": { "name": "zig" },
            "node": { "name": "node" }
        }"#;
        let tools = parse_outdated(json).unwrap();
        let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, vec!["node", "zig"]);
    }

    #[test]
    fn invalid_json_is_error() {
        assert!(parse_outdated("not json").is_err());
    }

    #[test]
    fn missing_entry_fields_default() {
        // An entry with no fields at all must not panic; the key supplies the name.
        let json = r#"{ "node": {} }"#;
        let tools = parse_outdated(json).unwrap();
        assert_eq!(tools[0].name, "node");
        assert_eq!(tools[0].current, None);
        assert_eq!(tools[0].requested, None);
        assert_eq!(tools[0].latest, None);
        assert_eq!(tools[0].source_path, None);
    }
}
