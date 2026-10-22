//! Application settings persistence (theme + locale).
//!
//! This module holds the small, user-facing settings blob that the `mise-hub`
//! GUI writes out and reads back: which theme is selected and which locale the
//! UI is rendered in.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// User-facing application settings persisted to disk as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    /// The selected theme name, e.g. `"Default Light"` or `"Default Dark"`.
    pub theme: String,
    /// The selected locale, e.g. `"en"` or `"zh-CN"`.
    pub locale: String,
    /// URL of the update-check manifest (see `mise_core::update`). Empty
    /// (the default, also what pre-M5 settings files deserialize to) means
    /// update checking is disabled.
    #[serde(default)]
    pub update_manifest_url: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "Default Light".into(),
            locale: "en".into(),
            update_manifest_url: String::new(),
        }
    }
}

impl AppSettings {
    /// Load settings from `path`, falling back to [`AppSettings::default`] when
    /// the file does not exist.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    /// Persist settings to `path`, creating any missing parent directories.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// The platform-specific path to the settings file, or `settings.json` in the
/// current directory when the platform config directory cannot be resolved.
pub fn default_settings_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "mise-hub", "mise-hub")
        .map(|d| d.config_dir().join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let s = AppSettings::load(&dir.path().join("settings.json")).unwrap();
        assert_eq!(s.theme, "Default Light");
        assert_eq!(s.locale, "en");
        assert_eq!(s.update_manifest_url, "");
    }

    #[test]
    fn pre_m5_file_without_update_url_deserializes_to_disabled() {
        // Settings written before M5 lack the field; loading one must not
        // error and must leave update checking disabled, not invent a URL.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"theme": "Default Dark", "locale": "zh-CN"}"#).unwrap();
        let s = AppSettings::load(&path).unwrap();
        assert_eq!(s.theme, "Default Dark");
        assert_eq!(s.locale, "zh-CN");
        assert_eq!(s.update_manifest_url, "");
    }

    #[test]
    fn roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let s = AppSettings {
            theme: "Default Dark".into(),
            locale: "zh-CN".into(),
            update_manifest_url: "https://example.com/manifest.json".into(),
        };
        s.save(&path).unwrap();
        assert_eq!(AppSettings::load(&path).unwrap(), s);
    }
}
