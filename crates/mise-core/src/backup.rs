//! Pre-write backups of config files (spec §5.4).
//!
//! Before any config-writing mise command (`use`, `set`, `unset`), the app
//! copies the target file into an app-managed backup directory. Writes
//! themselves still go ONLY through mise commands — this module is a plain
//! read+copy of the existing file, never a write path for config content.

use crate::error::CoreError;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A monotonic-ish counter suffix so two backups of the same file within the
/// same millisecond still get unique names (an atomic counter shared by
/// process; timestamps alone cannot guarantee uniqueness).
static BACKUP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Copy `path` into `dir` with a timestamped name; `Ok(None)` when the file
/// does not exist yet (nothing to back up). Creates `dir` if missing.
///
/// Backup name: `<stem>.<unix_millis>.<ext>` (e.g. `mise.1729450000123.toml`),
/// plus a counter suffix on collision, so repeated backups never overwrite.
pub fn backup_file(path: &Path, dir: &Path) -> Result<Option<PathBuf>, CoreError> {
    if !path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".into());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "toml".into());
    let n = BACKUP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut dest = dir.join(format!("{stem}.{millis}.{n}.{ext}"));
    // Without the counter (e.g. counter reset) a name collision is still
    // impossible to accept: bump the suffix until the name is free.
    while dest.exists() {
        let n = BACKUP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        dest = dir.join(format!("{stem}.{millis}.{n}.{ext}"));
    }
    std::fs::copy(path, &dest)?;
    Ok(Some(dest))
}

/// Platform-appropriate backup directory: the app's data dir + `backups`,
/// falling back to `backups` relative to the current directory when the
/// platform directory cannot be resolved (same policy as
/// `default_settings_path`).
pub fn default_backup_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "mise-hub", "mise-hub")
        .map(|d| d.data_dir().join("backups"))
        .unwrap_or_else(|| PathBuf::from("backups"))
}

/// mise's global config path (cross-platform): `<home>/.config/mise/config.toml`.
/// mise uses this location on all platforms it supports.
pub fn global_config_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| {
        d.home_dir()
            .join(".config")
            .join("mise")
            .join("config.toml")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_existing_file_with_identical_bytes() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let file = src.path().join("mise.toml");
        std::fs::write(&file, b"[tools]\nnode = \"20\"\n").unwrap();
        let backup = backup_file(&file, dst.path()).unwrap().unwrap();
        assert!(backup.is_file());
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            std::fs::read(&file).unwrap()
        );
        assert_eq!(backup.extension().unwrap(), "toml");
    }

    #[test]
    fn two_backups_never_overwrite() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let file = src.path().join("mise.toml");
        std::fs::write(&file, b"a").unwrap();
        let b1 = backup_file(&file, dst.path()).unwrap().unwrap();
        let b2 = backup_file(&file, dst.path()).unwrap().unwrap();
        assert_ne!(b1, b2);
        assert!(b1.is_file() && b2.is_file());
    }

    #[test]
    fn missing_file_is_none() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = backup_file(&src.path().join("nope.toml"), dst.path()).unwrap();
        assert!(out.is_none());
        assert_eq!(
            std::fs::read_dir(dst.path()).unwrap().count(),
            0,
            "no dir churn for a missing source"
        );
    }

    #[test]
    fn creates_nested_destination_dir() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let deep = dst.path().join("a").join("b").join("backups");
        let file = src.path().join("mise.toml");
        std::fs::write(&file, b"x").unwrap();
        let backup = backup_file(&file, &deep).unwrap().unwrap();
        assert!(deep.is_dir());
        assert!(backup.is_file());
    }
}
