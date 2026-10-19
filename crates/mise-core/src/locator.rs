use std::path::{Path, PathBuf};

pub fn locate_in(path_dirs: &[PathBuf], candidates: &[PathBuf]) -> Option<PathBuf> {
    for dir in path_dirs {
        let candidate = dir.join(if cfg!(windows) { "mise.exe" } else { "mise" });
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    candidates.iter().find(|p| is_executable(p)).cloned()
}

pub fn default_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        out.push(home.join(".local/bin/mise"));
        out.push(home.join(".cargo/bin/mise"));
    }
    out.push(PathBuf::from("/opt/homebrew/bin/mise"));
    out.push(PathBuf::from("/usr/local/bin/mise"));
    out
}

pub fn locate_mise() -> Option<PathBuf> {
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    locate_in(&path_dirs, &default_candidates())
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_first_existing_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let mise = dir.path().join("mise");
        std::fs::write(&mise, "").unwrap();

        let found = locate_in(&[], &[dir.path().join("missing"), mise.clone()]);

        assert_eq!(found, Some(mise));
    }

    #[test]
    fn returns_none_when_nothing_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(locate_in(&[], &[dir.path().join("missing")]), None);
    }
}
