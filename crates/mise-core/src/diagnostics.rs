//! Startup / diagnostics capability probes (M5).
//!
//! The GUI depends on a handful of `mise` JSON-producing subcommands whose
//! availability varies by mise build and version (AGENTS.md version-
//! compatibility rule). [`Mise::diagnostics`] runs each of them once,
//! read-only and in global scope, and records success or the RAW failure
//! text so the settings page can show exactly what is missing instead of a
//! generic "something is broken".

use std::path::PathBuf;

/// The outcome of probing one `mise` subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// Short subcommand name, stable UI key: `"registry"`, `"ls"`,
    /// `"config"`, `"env"`, `"tasks"`, `"outdated"`, `"ls-remote"`.
    pub name: &'static str,
    /// Whether the probe command exited 0. This only records that the CLI
    /// ran — output parsing stays with the caller.
    pub ok: bool,
    /// `true` for probes that reach the network (`ls-remote`). The settings
    /// page labels these differently so a failure is not mistaken for a
    /// missing feature.
    pub network: bool,
    /// The original error text (including mise's stderr) when `!ok`.
    pub detail: Option<String>,
}

/// A full diagnostics snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsReport {
    /// The resolved mise executable path the app is using.
    pub program: PathBuf,
    /// `mise --version`'s first token; `None` when the command failed — the
    /// probes still ran, a broken `--version` must not blank the page.
    pub version: Option<String>,
    /// One entry per probe, in declaration order.
    pub probes: Vec<Probe>,
}

/// Every JSON-producing subcommand the app depends on, with the argv used
/// to probe it and whether it needs the network. Probes are GLOBAL-scope
/// (no `-C`): diagnostics must not depend on, trust, or modify any project
/// config.
pub const PROBES: &[(&str, &[&str], bool)] = &[
    ("registry", &["registry", "--json"], false),
    ("ls", &["ls", "--json"], false),
    ("config", &["config", "ls", "--json"], false),
    ("env", &["env", "--json-extended"], false),
    ("tasks", &["tasks", "ls", "--json"], false),
    ("outdated", &["outdated", "--json"], false),
    ("ls-remote", &["ls-remote", "node", "--json"], true),
];

/// A neutral working directory for the probes: the user's home directory,
/// where mise's GLOBAL config lives.
///
/// Without an explicit cwd the child would inherit the app process's cwd,
/// and mise resolves its config by walking up from there — an unrelated
/// project config (or an untrusted one) would then turn healthy
/// capabilities red. The home directory is neutral with respect to
/// PROJECT configs; note that a home-level `mise.toml` would still be
/// discovered (that file *is* a config the user keeps next to their
/// global config), and that the global config is deliberately in scope:
/// a global-scope probe should see the global configuration.
pub fn probe_cwd() -> PathBuf {
    match directories::UserDirs::new() {
        Some(dirs) => dirs.home_dir().to_path_buf(),
        None => std::env::temp_dir(),
    }
}

/// How long a single probe may take before it is reported as timed out.
/// The network probe (`ls-remote`) is the slow one; without a bound a
/// hanging request would keep the 「重新探测」 button disabled forever.
pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
    use crate::mise::Mise;
    use crate::runner::{
        CommandOutput, CommandRequest, CommandRunner, StreamEvent, StreamingRunner,
    };
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// A runner that records every request (argv AND working directory)
    /// and fails exactly the commands whose joined args are listed in
    /// `fail`, echoing a marker on stderr.
    struct FakeRunner {
        seen: Arc<Mutex<Vec<CommandRequest>>>,
        fail: Vec<String>,
        slow: Vec<String>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError> {
            self.seen.lock().unwrap().push(req.clone());
            let joined = req.args.join(" ");
            if self.slow.contains(&joined) {
                // Longer than the timeout the test passes in.
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            if self.fail.contains(&joined) {
                return Ok(CommandOutput {
                    code: 1,
                    stdout: String::new(),
                    stderr: format!("boom: {joined}"),
                });
            }
            Ok(CommandOutput {
                code: 0,
                stdout: "2026.10.15 macos-arm64".into(),
                stderr: String::new(),
            })
        }
    }

    struct NoopStreamer;
    impl StreamingRunner for NoopStreamer {
        fn spawn(
            &self,
            _req: CommandRequest,
        ) -> Result<std::sync::mpsc::Receiver<StreamEvent>, CoreError> {
            unreachable!("diagnostics never streams")
        }
    }

    fn mise_with(fail: Vec<String>) -> (Mise, Arc<Mutex<Vec<CommandRequest>>>) {
        mise_with_behavior(fail, vec![])
    }

    /// A mise whose listed commands hang (used for the timeout test).
    fn mise_with_slow(slow: Vec<String>) -> (Mise, Arc<Mutex<Vec<CommandRequest>>>) {
        mise_with_behavior(vec![], slow)
    }

    fn mise_with_behavior(
        fail: Vec<String>,
        slow: Vec<String>,
    ) -> (Mise, Arc<Mutex<Vec<CommandRequest>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mise = Mise::new(
            PathBuf::from("/usr/local/bin/mise"),
            Arc::new(FakeRunner {
                seen: seen.clone(),
                fail,
                slow,
            }),
            Arc::new(NoopStreamer),
        );
        (mise, seen)
    }

    #[test]
    fn program_exposes_the_resolved_path() {
        let (mise, _) = mise_with(vec![]);
        assert_eq!(mise.program(), Path::new("/usr/local/bin/mise"));
    }

    #[test]
    fn every_probe_runs_globally_without_dash_c_and_in_order() {
        let (mise, seen) = mise_with(vec![]);
        let report = mise.diagnostics();

        assert_eq!(report.program, PathBuf::from("/usr/local/bin/mise"));
        assert_eq!(report.version.as_deref(), Some("2026.10.15"));

        // --version runs first, then the probes in declaration order.
        let mut expected: Vec<Vec<String>> = vec![vec!["--version".into()]];
        expected.extend(
            PROBES
                .iter()
                .map(|(_, args, _)| args.iter().map(|s| s.to_string()).collect()),
        );
        let seen = seen.lock().unwrap();
        let actual: Vec<Vec<String>> = seen.iter().map(|req| req.args.clone()).collect();
        assert_eq!(actual, expected);

        assert_eq!(report.probes.len(), PROBES.len());
        assert!(report.probes.iter().all(|p| p.ok && p.detail.is_none()));
        assert!(
            !seen.iter().any(|req| req.args.contains(&"-C".to_string())),
            "diagnostics are global-scope: no -C anywhere"
        );
    }

    /// Every probe must run with an explicit, neutral working directory.
    /// Inheriting the app's cwd would make the probe result depend on
    /// wherever the GUI was started: an unrelated (or untrusted) project
    /// config there would turn healthy capabilities red.
    #[test]
    fn every_probe_runs_in_a_neutral_working_directory() {
        let (mise, seen) = mise_with(vec![]);
        mise.diagnostics();

        let expected = probe_cwd();
        let app_cwd = std::env::current_dir().ok();
        for req in seen.lock().unwrap().iter() {
            assert_eq!(
                req.cwd.as_deref(),
                Some(expected.as_path()),
                "probe {:?} must not inherit the app's cwd",
                req.args
            );
            assert!(
                req.cwd.is_some(),
                "probe {:?} must carry an explicit cwd",
                req.args
            );
            assert_ne!(
                req.cwd.as_deref(),
                app_cwd.as_deref(),
                "probe {:?} must not run in the app's own working directory",
                req.args
            );
        }
    }

    /// A probe that hangs must not hang the whole report: it is reported
    /// as failed with a timeout note, and the other probes still land.
    #[test]
    fn a_hanging_probe_is_reported_as_timed_out() {
        let (mise, seen) = mise_with_slow(vec!["ls --json".into()]);
        let report = mise.diagnostics_with_timeout(std::time::Duration::from_millis(50));

        let ls = report.probes.iter().find(|p| p.name == "ls").unwrap();
        assert!(!ls.ok, "the slow probe must not report success");
        let detail = ls.detail.as_deref().unwrap();
        assert!(
            detail.contains("timed out"),
            "timeout must be visible: {detail}"
        );

        // The other probes still ran (and the version was read).
        assert!(report.probes.iter().filter(|p| p.ok).count() >= PROBES.len() - 1);
        assert!(!seen.lock().unwrap().is_empty());
    }

    #[test]
    fn only_ls_remote_is_marked_as_network() {
        let (mise, _) = mise_with(vec![]);
        let report = mise.diagnostics();
        let network: Vec<&str> = report
            .probes
            .iter()
            .filter(|p| p.network)
            .map(|p| p.name)
            .collect();
        assert_eq!(network, vec!["ls-remote"]);
    }

    #[test]
    fn a_failing_probe_carries_the_raw_error_and_others_still_run() {
        let (mise, seen) = mise_with(vec!["ls --json".into()]);
        let report = mise.diagnostics();

        let ls = report.probes.iter().find(|p| p.name == "ls").unwrap();
        assert!(!ls.ok);
        let detail = ls.detail.as_deref().unwrap();
        assert!(
            detail.contains("boom: ls --json"),
            "raw stderr kept: {detail}"
        );

        let env = report.probes.iter().find(|p| p.name == "env").unwrap();
        assert!(env.ok, "one failure must not abort the other probes");
        // All probes ran despite the failure.
        assert_eq!(seen.lock().unwrap().len(), 1 + PROBES.len());
    }

    #[test]
    fn version_failure_keeps_probes_running_with_none_version() {
        let (mise, seen) = mise_with(vec!["--version".into()]);
        let report = mise.diagnostics();
        assert_eq!(report.version, None);
        assert!(report.probes.iter().all(|p| p.ok));
        assert_eq!(seen.lock().unwrap().len(), 1 + PROBES.len());
    }
}
