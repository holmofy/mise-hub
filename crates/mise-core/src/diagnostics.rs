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

    /// A runner that records every argv and fails exactly the commands
    /// whose joined args are listed in `fail`, echoing a marker on stderr.
    struct FakeRunner {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
        fail: Vec<String>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            let joined = req.args.join(" ");
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

    fn mise_with(fail: Vec<String>) -> (Mise, Arc<Mutex<Vec<Vec<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mise = Mise::new(
            PathBuf::from("/usr/local/bin/mise"),
            Arc::new(FakeRunner {
                seen: seen.clone(),
                fail,
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
        assert_eq!(*seen.lock().unwrap(), expected);

        assert_eq!(report.probes.len(), PROBES.len());
        assert!(report.probes.iter().all(|p| p.ok && p.detail.is_none()));
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|args| args.contains(&"-C".to_string())),
            "diagnostics are global-scope: no -C anywhere"
        );
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
