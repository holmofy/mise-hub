use crate::diagnostics::{DiagnosticsReport, PROBE_TIMEOUT, PROBES, Probe, probe_cwd};
use crate::env::{EnvDeclaration, EnvVar, parse_env_extended, parse_set_list};
use crate::error::CoreError;
use crate::manage::{UsePreview, parse_use_dry_run};
use crate::outdated::{OutdatedTool, parse_outdated};
use crate::registry::parse_registry;
use crate::remote::parse_remote_versions;
use crate::runner::{CommandOutput, CommandRequest, CommandRunner, StreamEvent, StreamingRunner};
use crate::search::{DeepSearchHit, parse_search_table};
use crate::tasks::{Task, parse_tasks};
use crate::tools::InstalledSet;
use crate::types::{RemoteVersion, ToolEntry, ToolInfo};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

pub struct Mise {
    program: PathBuf,
    runner: Arc<dyn CommandRunner>,
    streamer: Arc<dyn StreamingRunner>,
}

/// The first whitespace-separated token of `mise --version`'s output —
/// e.g. `"2026.10.15"` from `"2026.10.15 macos-arm64 (sha)"`.
fn parse_version(stdout: &str) -> String {
    stdout
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

impl Mise {
    pub fn new(
        program: PathBuf,
        runner: Arc<dyn CommandRunner>,
        streamer: Arc<dyn StreamingRunner>,
    ) -> Self {
        Self {
            program,
            runner,
            streamer,
        }
    }

    /// The version reported by `mise --version`.
    ///
    /// Runs in the neutral probe directory like the capability probes, so
    /// the app's own version display never depends on the directory the
    /// GUI was started from (a deleted working directory would otherwise
    /// make the child process fail to spawn at all).
    pub fn version(&self) -> Result<String, CoreError> {
        let cwd = probe_cwd();
        let out = self.run_in(&["--version"], None, Some(&cwd))?;
        Ok(parse_version(&out.stdout))
    }

    /// The resolved path of the `mise` executable this instance calls.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Probe every command in [`PROBES`] and collect a report. Slow (it
    /// spawns up to eight processes, one of them network-bound) — callers
    /// must run it on a background thread. Never panics: every failure
    /// becomes `Probe.detail` with the original message.
    ///
    /// Every probe runs with an explicit neutral working directory
    /// ([`probe_cwd`]) instead of inheriting the app's cwd: mise resolves
    /// its config by walking up from the cwd, so probing from wherever
    /// the GUI was started could turn healthy capabilities red because of
    /// an unrelated project config (and could demand trusting it).
    pub fn diagnostics(&self) -> DiagnosticsReport {
        self.diagnostics_with_timeout(PROBE_TIMEOUT)
    }

    /// [`Mise::diagnostics`] with an explicit per-probe timeout. A probe
    /// that outlives it is reported as failed ("timed out after Ns")
    /// instead of blocking the whole report — the network probe is the
    /// one that can hang.
    pub fn diagnostics_with_timeout(&self, timeout: std::time::Duration) -> DiagnosticsReport {
        let cwd = probe_cwd();
        let version = self
            .run_probe(&["--version"], Some(&cwd), timeout)
            .ok()
            .map(|out| parse_version(&out.stdout));
        let probes = PROBES
            .iter()
            .map(
                |&(name, args, network)| match self.run_probe(args, Some(&cwd), timeout) {
                    Ok(_) => Probe {
                        name,
                        ok: true,
                        network,
                        detail: None,
                    },
                    Err(e) => Probe {
                        name,
                        ok: false,
                        network,
                        detail: Some(e.to_string()),
                    },
                },
            )
            .collect();
        DiagnosticsReport {
            program: self.program.clone(),
            version,
            probes,
        }
    }

    /// Run one command with a hard time bound. The command runs on a
    /// helper thread so a hanging child cannot block the caller; the
    /// helper may outlive the timeout (the child process is not killed),
    /// but its result is then discarded.
    fn run_probe(
        &self,
        args: &[&str],
        cwd: Option<&Path>,
        timeout: std::time::Duration,
    ) -> Result<CommandOutput, CoreError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let program = self.program.clone();
        let runner = self.runner.clone();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let cwd = cwd.map(|dir| dir.to_path_buf());
        std::thread::spawn(move || {
            let result = runner.run(&CommandRequest { program, args, cwd });
            let _ = tx.send(result);
        });
        match rx.recv_timeout(timeout) {
            // A non-zero exit is a failure, exactly as in `Mise::run`:
            // the probe records the RAW stderr, not a "success".
            Ok(Ok(out)) if out.code != 0 => Err(CoreError::CommandFailed {
                code: out.code,
                stderr: out.stderr,
            }),
            Ok(Ok(out)) => Ok(out),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CoreError::Other(format!(
                "timed out after {}s",
                timeout.as_secs()
            ))),
        }
    }

    pub fn registry(&self) -> Result<Vec<ToolEntry>, CoreError> {
        let out = self.run(&["registry", "--json"], None)?;
        parse_registry(&out.stdout)
    }

    pub fn installed(&self, project_root: Option<&Path>) -> Result<InstalledSet, CoreError> {
        let out = self.run(&["ls", "--json"], project_root)?;
        InstalledSet::parse(&out.stdout)
    }

    pub fn remote_versions(
        &self,
        tool: &str,
        project_root: Option<&Path>,
    ) -> Result<Vec<RemoteVersion>, CoreError> {
        let out = self.run(&["ls-remote", tool, "--json"], project_root)?;
        parse_remote_versions(&out.stdout)
    }

    pub fn install_stream(
        &self,
        tool: &str,
        version: &str,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let spec = format!("{tool}@{version}");
        let mut args = self.base_args(project_root);
        args.extend(["install".into(), spec]);
        self.spawn_stream(args)
    }

    /// Rows of `mise outdated --json`, sorted by tool name.
    ///
    /// `global == true` checks only the global config's requests (same scope
    /// as `mise upgrade --global`, which `mise outdated --help` documents as
    /// `--global`), dropping `-C`; otherwise the run is scoped to
    /// `project_root` via `-C`.
    pub fn outdated(
        &self,
        global: bool,
        project_root: Option<&Path>,
    ) -> Result<Vec<OutdatedTool>, CoreError> {
        let mut args = self.base_or_global(global, project_root);
        args.extend(["outdated".into(), "--json".into()]);
        if global {
            args.push("--global".into());
        }
        let fullargs = args;
        let out = self.runner.run(&CommandRequest {
            program: self.program.clone(),
            args: fullargs,
            cwd: None,
        })?;
        if out.code != 0 {
            return Err(CoreError::CommandFailed {
                code: out.code,
                stderr: out.stderr,
            });
        }
        parse_outdated(&out.stdout)
    }

    /// Details of `mise tool <name> --json`.
    pub fn tool_info(
        &self,
        tool: &str,
        project_root: Option<&Path>,
    ) -> Result<ToolInfo, CoreError> {
        let out = self.run(&["tool", tool, "--json"], project_root)?;
        serde_json::from_str(&out.stdout).map_err(CoreError::Json)
    }

    /// Streamed `mise upgrade [--bump] [--global] [spec]`.
    ///
    /// `global == true` adds `--global` and drops `-C`; `spec == None`
    /// upgrades everything.
    pub fn upgrade_stream(
        &self,
        spec: Option<&str>,
        bump: bool,
        global: bool,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = if global {
            vec![]
        } else {
            self.base_args(project_root)
        };
        args.push("upgrade".into());
        if bump {
            args.push("--bump".into());
        }
        if global {
            args.push("--global".into());
        }
        if let Some(spec) = spec {
            args.push(spec.into());
        }
        self.spawn_stream(args)
    }

    /// Streamed `mise uninstall <spec>`.
    pub fn uninstall_stream(
        &self,
        spec: &str,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_args(project_root);
        args.extend(["uninstall".into(), spec.into()]);
        self.spawn_stream(args)
    }

    /// Text preview of `mise use --dry-run <spec>`.
    pub fn use_dry_run(
        &self,
        spec: &str,
        global: bool,
        project_root: Option<&Path>,
    ) -> Result<UsePreview, CoreError> {
        let mut args = self.base_or_global(global, project_root);
        args.extend(["use".into(), "--dry-run".into()]);
        if global {
            args.push("--global".into());
        }
        if !spec.is_empty() {
            args.push(spec.into());
        }
        let out = self.runner.run(&CommandRequest {
            program: self.program.clone(),
            args,
            cwd: None,
        })?;
        if out.code != 0 {
            return Err(CoreError::CommandFailed {
                code: out.code,
                stderr: out.stderr,
            });
        }
        parse_use_dry_run(&out.stdout)
    }

    /// Streamed `mise use [--pin] <spec>` (no `--dry-run`).
    pub fn use_apply_stream(
        &self,
        spec: &str,
        global: bool,
        pin: bool,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_or_global(global, project_root);
        args.push("use".into());
        if pin {
            args.push("--pin".into());
        }
        if global {
            args.push("--global".into());
        }
        if !spec.is_empty() {
            args.push(spec.into());
        }
        self.spawn_stream(args)
    }

    /// Rows of `mise tasks ls --json`, in mise's own order. Task names
    /// with backend-ish prefixes or colons (file tasks) come back verbatim.
    pub fn tasks(&self, project_root: Option<&Path>) -> Result<Vec<Task>, CoreError> {
        let out = self.run(&["tasks", "ls", "--json"], project_root)?;
        parse_tasks(&out.stdout)
    }

    /// Streamed `mise [-C dir] run <task>`. The task name is passed
    /// verbatim; the panel is responsible for surfacing non-zero exits.
    pub fn run_task_stream(
        &self,
        task: &str,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_args(project_root);
        args.extend(["run".into(), task.into()]);
        self.spawn_stream(args)
    }

    /// Streamed `mise [-C dir] watch <task>`.
    pub fn watch_task_stream(
        &self,
        task: &str,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_args(project_root);
        args.extend(["watch".into(), task.into()]);
        self.spawn_stream(args)
    }

    /// `mise trust <dir>`; non-zero exit becomes `CommandFailed`.
    pub fn trust(&self, dir: &Path) -> Result<(), CoreError> {
        self.run(&["trust", &dir.to_string_lossy()], None)?;
        Ok(())
    }

    /// Online, user-triggered `mise search --no-header --all <query>` wide
    /// search. The query is passed verbatim (backend prefixes like
    /// `cargo:x` must stay intact); non-zero exit becomes `CommandFailed`.
    pub fn deep_search(
        &self,
        query: &str,
        project_root: Option<&Path>,
    ) -> Result<Vec<DeepSearchHit>, CoreError> {
        let out = self.run(&["search", "--no-header", "--all", query], project_root)?;
        parse_search_table(&out.stdout)
    }

    /// Rows of `mise env --json-extended`, sorted by name. Fails (including
    /// `not trusted`) when the project config is not trusted.
    pub fn env_extended(&self, project_root: Option<&Path>) -> Result<Vec<EnvVar>, CoreError> {
        let out = self.run(&["env", "--json-extended"], project_root)?;
        parse_env_extended(&out.stdout)
    }

    /// Rows of the plain-text `mise set` listing (declarations only; no JSON
    /// form exists). `--no-redact` is required: mise redacts variables declared
    /// with `redact = true` as the literal `[redacted]` by default, which
    /// would otherwise destroy secrets when an ambiguous value is edited
    /// back. Ambiguous rows (value that looks like a path) should be re-read
    /// via `env_value`.
    pub fn env_declarations(
        &self,
        project_root: Option<&Path>,
    ) -> Result<Vec<EnvDeclaration>, CoreError> {
        let out = self.run(&["set", "--no-redact"], project_root)?;
        parse_set_list(&out.stdout)
    }

    /// `mise set <NAME>`: prints only that value (the documented fallback for
    /// ambiguous `mise set` table rows). Returns trimmed stdout.
    pub fn env_value(&self, name: &str, project_root: Option<&Path>) -> Result<String, CoreError> {
        let out = self.run(&["set", name], project_root)?;
        Ok(out.stdout.trim().to_string())
    }

    /// Streamed `mise [-C dir] set [--global] NAME=VALUE`. `--global` must
    /// come AFTER the subcommand. `NAME=VALUE` is passed as ONE argv element
    /// so values with spaces stay intact.
    pub fn set_env_stream(
        &self,
        name: &str,
        value: &str,
        global: bool,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_or_global(global, project_root);
        args.push("set".into());
        if global {
            args.push("--global".into());
        }
        args.push(format!("{name}={value}"));
        self.spawn_stream(args)
    }

    /// Streamed `mise [-C dir] set --file <file> NAME=VALUE`. Targets ONE
    /// exact config file (the declaration's own `source`), so an edit to a
    /// global-sourced row can never land in the project file. `--file` is the
    /// documented flag of `mise set`; `NAME=VALUE` stays ONE argv element.
    pub fn set_env_file_stream(
        &self,
        name: &str,
        value: &str,
        file: &Path,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_or_global(false, project_root);
        args.push("set".into());
        args.push("--file".into());
        args.push(file.to_string_lossy().into_owned());
        args.push(format!("{name}={value}"));
        self.spawn_stream(args)
    }

    /// Streamed `mise [-C dir] unset --file <file> NAME`. Same contract as
    /// `set_env_file_stream`, for the delete action.
    pub fn unset_env_file_stream(
        &self,
        name: &str,
        file: &Path,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_or_global(false, project_root);
        args.push("unset".into());
        args.push("--file".into());
        args.push(file.to_string_lossy().into_owned());
        args.push(name.into());
        self.spawn_stream(args)
    }

    /// Streamed `mise [-C dir] unset [--global] NAME`. `--global` must come
    /// AFTER the subcommand.
    pub fn unset_env_stream(
        &self,
        name: &str,
        global: bool,
        project_root: Option<&Path>,
    ) -> Result<Receiver<StreamEvent>, CoreError> {
        let mut args = self.base_or_global(global, project_root);
        args.push("unset".into());
        if global {
            args.push("--global".into());
        }
        args.push(name.into());
        self.spawn_stream(args)
    }

    /// `-C <dir>` for project scope, empty for global scope (callers add the
    /// `--global` flag themselves). NOTE: different mise subcommands accept
    /// `--global` in different positions; for `outdated`, `upgrade`, `use`,
    /// `set`, and `unset` it is a subcommand flag and must be placed AFTER
    /// the subcommand, so `base_or_global` never adds it.
    fn base_or_global(&self, global: bool, project_root: Option<&Path>) -> Vec<String> {
        if global {
            vec![]
        } else {
            self.base_args(project_root)
        }
    }

    fn spawn_stream(&self, args: Vec<String>) -> Result<Receiver<StreamEvent>, CoreError> {
        self.streamer.spawn(CommandRequest {
            program: self.program.clone(),
            args,
            cwd: None,
        })
    }

    fn base_args(&self, project_root: Option<&Path>) -> Vec<String> {
        match project_root {
            Some(dir) => vec!["-C".into(), dir.to_string_lossy().into_owned()],
            None => vec![],
        }
    }

    fn run(&self, args: &[&str], project_root: Option<&Path>) -> Result<CommandOutput, CoreError> {
        self.run_in(args, project_root, None)
    }

    /// Like [`Mise::run`], but with an explicit working directory for the
    /// child process. `cwd: None` means "inherit the app's cwd", which
    /// makes the result depend on where the GUI was started — the
    /// diagnostics probes therefore always pass a neutral directory.
    fn run_in(
        &self,
        args: &[&str],
        project_root: Option<&Path>,
        cwd: Option<&Path>,
    ) -> Result<CommandOutput, CoreError> {
        let mut full = self.base_args(project_root);
        full.extend(args.iter().map(|s| s.to_string()));
        let out = self.runner.run(&CommandRequest {
            program: self.program.clone(),
            args: full,
            cwd: cwd.map(|dir| dir.to_path_buf()),
        })?;
        if out.code != 0 {
            return Err(CoreError::CommandFailed {
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{CommandOutput, CommandRequest};
    use std::sync::{Arc, Mutex};

    struct RecordingRunner {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
        stdout: String,
    }

    impl CommandRunner for RecordingRunner {
        fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(CommandOutput {
                code: 0,
                stdout: self.stdout.clone(),
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
            Ok(std::sync::mpsc::channel().1)
        }
    }

    fn mise(stdout: &str) -> (Mise, Arc<Mutex<Vec<Vec<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = Arc::new(RecordingRunner {
            seen: seen.clone(),
            stdout: stdout.into(),
        });
        (
            Mise::new("mise".into(), runner, Arc::new(NoopStreamer)),
            seen,
        )
    }

    #[test]
    fn passes_project_dir_before_subcommand() {
        let (m, seen) = mise("{}");
        m.installed(Some(Path::new("/proj"))).unwrap();
        assert_eq!(seen.lock().unwrap()[0], vec!["-C", "/proj", "ls", "--json"]);
    }

    #[test]
    fn omits_project_dir_for_global() {
        let (m, seen) = mise("{}");
        m.installed(None).unwrap();
        assert_eq!(seen.lock().unwrap()[0], vec!["ls", "--json"]);
    }

    #[test]
    fn parses_version_first_token() {
        let (m, seen) = mise("2026.10.13 macos-arm64 (2026-10-17)\n");
        assert_eq!(m.version().unwrap(), "2026.10.13");
        assert_eq!(seen.lock().unwrap()[0], vec!["--version"]);
    }

    #[test]
    fn tool_name_with_backend_prefix_is_verbatim() {
        let (m, seen) = mise("[]");
        let _ = m.remote_versions("npm:prettier", None);
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["ls-remote", "npm:prettier", "--json"]
        );
    }

    #[test]
    fn non_zero_exit_becomes_command_failed() {
        struct Failing;
        impl CommandRunner for Failing {
            fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
                Ok(CommandOutput {
                    code: 2,
                    stdout: String::new(),
                    stderr: "boom".into(),
                })
            }
        }
        let m = Mise::new("mise".into(), Arc::new(Failing), Arc::new(NoopStreamer));
        let err = m.registry().unwrap_err();
        assert!(matches!(err, CoreError::CommandFailed { code: 2, .. }));
    }

    // ---- M2-T1 ----

    struct RecordingStreamer {
        seen: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl StreamingRunner for RecordingStreamer {
        fn spawn(
            &self,
            req: CommandRequest,
        ) -> Result<std::sync::mpsc::Receiver<StreamEvent>, CoreError> {
            self.seen.lock().unwrap().push(req.args.clone());
            Ok(std::sync::mpsc::channel().1)
        }
    }

    type SeenLog = Arc<Mutex<Vec<Vec<String>>>>;

    fn mise_rec_stream(stdout: &str) -> (Mise, SeenLog, SeenLog) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stream_seen = Arc::new(Mutex::new(Vec::new()));
        let runner = Arc::new(RecordingRunner {
            seen: seen.clone(),
            stdout: stdout.into(),
        });
        (
            Mise::new(
                "mise".into(),
                runner,
                Arc::new(RecordingStreamer {
                    seen: stream_seen.clone(),
                }),
            ),
            seen,
            stream_seen,
        )
    }

    #[test]
    fn outdated_passes_dash_c_before_subcommand() {
        let (m, seen, _) = mise_rec_stream("{}");
        m.outdated(false, Some(Path::new("/proj"))).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C", "/proj", "outdated", "--json"]
        );
    }

    #[test]
    fn outdated_global_omits_dash_c_and_adds_flag() {
        // `mise outdated --help` (mise 2026.10.15) documents `--global`:
        // "Only check tools defined in the global config", matching the
        // `mise upgrade --global` scope, so the global listing and the
        // global upgrade run use the same scope.
        let (m, seen, _) = mise_rec_stream("{}");
        m.outdated(true, Some(Path::new("/proj"))).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["outdated", "--json", "--global"]
        );
    }

    #[test]
    fn tool_info_argv_and_parse() {
        let (m, seen, _) = mise_rec_stream(TOOL_INFO_JSON);
        let info = m
            .tool_info("npm:prettier", Some(Path::new("/proj")))
            .unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C", "/proj", "tool", "npm:prettier", "--json"]
        );
        assert_eq!(info.installed_versions, vec!["24.22.0"]);
        assert_eq!(
            info.config_source.as_ref().map(|s| s.kind.as_str()),
            Some("mise.toml")
        );
    }

    const TOOL_INFO_JSON: &str = r#"{
        "backend": "core:node",
        "description": null,
        "url": null,
        "installed_versions": ["24.22.0"],
        "requested_versions": ["24.22.0"],
        "active_versions": ["24.22.0"],
        "config_source": {
            "type": "mise.toml",
            "path": "/Users/x/.config/mise/config.toml"
        }
    }"#;

    #[test]
    fn upgrade_no_spec_upgrades_everything() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.upgrade_stream(None, false, false, Some(Path::new("/proj")));
        assert_eq!(stream.lock().unwrap()[0], vec!["-C", "/proj", "upgrade"]);
    }

    #[test]
    fn upgrade_with_spec_and_bump() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.upgrade_stream(
            Some("npm:prettier@3"),
            true,
            false,
            Some(Path::new("/proj")),
        );
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "upgrade", "--bump", "npm:prettier@3"]
        );
    }

    #[test]
    fn upgrade_global_drops_dash_c_and_adds_flag() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.upgrade_stream(Some("node"), false, true, Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["upgrade", "--global", "node"]
        );
    }

    #[test]
    fn uninstall_argv_keeps_spec_verbatim() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.uninstall_stream("npm:prettier@3", Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "uninstall", "npm:prettier@3"]
        );
    }

    #[test]
    fn use_dry_run_argv_and_non_zero_exit_is_error() {
        let (m, seen, _) = mise_rec_stream("");
        let _ = m
            .use_dry_run("node@20.11.0", false, Some(Path::new("/proj")))
            .unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C", "/proj", "use", "--dry-run", "node@20.11.0"]
        );
    }

    #[test]
    fn use_dry_run_global_swaps_dash_c_for_flag() {
        struct Failing;
        impl CommandRunner for Failing {
            fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
                Ok(CommandOutput {
                    code: 1,
                    stdout: String::new(),
                    stderr: "not trusted".into(),
                })
            }
        }
        let m = Mise::new("mise".into(), Arc::new(Failing), Arc::new(NoopStreamer));
        let err = m
            .use_dry_run("node@20", true, Some(Path::new("/proj")))
            .unwrap_err();
        assert!(matches!(err, CoreError::CommandFailed { code: 1, .. }));
    }

    #[test]
    fn use_dry_run_global_argv_includes_flag() {
        let (m, seen, _) = mise_rec_stream("");
        m.use_dry_run("node@20", true, None).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["use", "--dry-run", "--global", "node@20"]
        );
    }

    #[test]
    fn use_apply_argv_without_pin() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.use_apply_stream("node@20.11.0", false, false, Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "use", "node@20.11.0"]
        );
    }

    #[test]
    fn use_apply_argv_with_pin_and_global() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.use_apply_stream("node@20.11.0", true, true, Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["use", "--pin", "--global", "node@20.11.0"]
        );
    }

    #[test]
    fn trust_argv_and_exit_code() {
        let (m, seen, _) = mise_rec_stream("");
        m.trust(Path::new("/proj")).unwrap();
        assert_eq!(seen.lock().unwrap()[0], vec!["trust", "/proj"]);

        struct Failing;
        impl CommandRunner for Failing {
            fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
                Ok(CommandOutput {
                    code: 3,
                    stdout: String::new(),
                    stderr: "refused".into(),
                })
            }
        }
        let m = Mise::new("mise".into(), Arc::new(Failing), Arc::new(NoopStreamer));
        let err = m.trust(Path::new("/proj")).unwrap_err();
        assert!(matches!(err, CoreError::CommandFailed { code: 3, .. }));
    }

    // ---- M2-T4: deep search ----

    const SEARCH_TABLE: &str = "ripgrep                           ripgrep \
recursively searches directories ... https://github.com/BurntSushi/ripgrep\n";

    #[test]
    fn deep_search_argv_with_project() {
        let (m, seen, _) = mise_rec_stream(SEARCH_TABLE);
        let hits = m.deep_search("ripgrep", Some(Path::new("/proj"))).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C", "/proj", "search", "--no-header", "--all", "ripgrep"]
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "ripgrep");
    }

    #[test]
    fn deep_search_argv_without_project() {
        let (m, seen, _) = mise_rec_stream(SEARCH_TABLE);
        m.deep_search("ripgrep", None).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["search", "--no-header", "--all", "ripgrep"]
        );
    }

    #[test]
    fn deep_search_query_with_backend_prefix_is_verbatim() {
        let (m, seen, _) = mise_rec_stream(SEARCH_TABLE);
        m.deep_search("cargo:ripgrep", None).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["search", "--no-header", "--all", "cargo:ripgrep"]
        );
    }

    // ---- M3-T1: env ----

    const ENV_JSON: &str = r#"{"A": {"source": "/proj/mise.toml", "value": "1"}}"#;

    #[test]
    fn env_extended_argv_scoped_to_project() {
        let (m, seen, _) = mise_rec_stream(ENV_JSON);
        m.env_extended(Some(Path::new("/proj"))).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C", "/proj", "env", "--json-extended"]
        );
    }

    #[test]
    fn env_extended_argv_unscoped() {
        let (m, seen, _) = mise_rec_stream(ENV_JSON);
        assert_eq!(m.env_extended(None).unwrap().len(), 1);
        assert_eq!(seen.lock().unwrap()[0], vec!["env", "--json-extended"]);
    }

    #[test]
    fn env_declarations_argv_and_parse() {
        let (m, seen, _) = mise_rec_stream("A  1  /proj/mise.toml\n");
        let rows = m.env_declarations(Some(Path::new("/proj"))).unwrap();
        // `--no-redact` is required: mise redacts `redact = true` variables
        // by default, and the raw value is what declarations need.
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C", "/proj", "set", "--no-redact"]
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "A");
    }

    #[test]
    fn env_value_returns_trimmed_stdout() {
        // The documented fallback for ambiguous table rows.
        let (m, seen, _) = mise_rec_stream("x y z\n");
        let v = m.env_value("B", Some(Path::new("/proj"))).unwrap();
        assert_eq!(seen.lock().unwrap()[0], vec!["-C", "/proj", "set", "B"]);
        assert_eq!(v, "x y z");
    }

    #[test]
    fn set_env_stream_project_keeps_name_value_pair_as_one_arg() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.set_env_stream("B", "x y z", false, Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "set", "B=x y z"]
        );
    }

    #[test]
    fn set_env_stream_global_flag_after_subcommand() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.set_env_stream("NODE_ENV", "production", true, Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["set", "--global", "NODE_ENV=production"]
        );
    }

    #[test]
    fn unset_env_stream_project_attr() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.unset_env_stream("FOO", false, Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "unset", "FOO"]
        );
    }

    #[test]
    fn unset_env_stream_global_flag_after_subcommand() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.unset_env_stream("FOO", true, None);
        assert_eq!(stream.lock().unwrap()[0], vec!["unset", "--global", "FOO"]);
    }

    #[test]
    fn set_env_file_stream_project_context_carries_dash_c() {
        // Editing a declaration of an exact file inside a project context:
        // `-C` first (AGENTS.md rule 2), then the subcommand, `--file` and
        // the NAME=VALUE pair as ONE argv element.
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.set_env_file_stream(
            "B",
            "x y z",
            Path::new("/proj/mise.toml"),
            Some(Path::new("/proj")),
        );
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "set", "--file", "/proj/mise.toml", "B=x y z"]
        );
    }

    #[test]
    fn set_env_file_stream_global_context_has_no_dash_c() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.set_env_file_stream(
            "NODE_ENV",
            "production",
            Path::new("/home/x/.config/mise/config.toml"),
            None,
        );
        assert_eq!(
            stream.lock().unwrap()[0],
            vec![
                "set",
                "--file",
                "/home/x/.config/mise/config.toml",
                "NODE_ENV=production"
            ]
        );
    }

    #[test]
    fn unset_env_file_stream_project_context_carries_dash_c() {
        let (m, _, stream) = mise_rec_stream("");
        let _ =
            m.unset_env_file_stream("B", Path::new("/proj/mise.toml"), Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "unset", "--file", "/proj/mise.toml", "B"]
        );
    }

    #[test]
    fn unset_env_file_stream_global_context_has_no_dash_c() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.unset_env_file_stream("A", Path::new("/g.toml"), None);
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["unset", "--file", "/g.toml", "A"]
        );
    }

    #[test]
    fn env_non_zero_exit_is_command_failed() {
        // M2-T5: untrusted configs make env/set fail with "not trusted".
        struct Failing;
        impl CommandRunner for Failing {
            fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
                Ok(CommandOutput {
                    code: 1,
                    stdout: String::new(),
                    stderr: "config not trusted".into(),
                })
            }
        }
        let m = Mise::new("mise".into(), Arc::new(Failing), Arc::new(NoopStreamer));
        assert!(m.env_extended(None).is_err());
        assert!(m.env_declarations(None).is_err());
        assert!(m.env_value("A", None).is_err());
        assert!(matches!(
            m.env_extended(None).unwrap_err(),
            CoreError::CommandFailed { code: 1, .. }
        ));
    }

    #[test]
    fn deep_search_non_zero_exit_is_command_failed() {
        struct Failing;
        impl CommandRunner for Failing {
            fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
                Ok(CommandOutput {
                    code: 2,
                    stdout: String::new(),
                    stderr: "offline".into(),
                })
            }
        }
        let m = Mise::new("mise".into(), Arc::new(Failing), Arc::new(NoopStreamer));
        let err = m.deep_search("ripgrep", None).unwrap_err();
        assert!(matches!(err, CoreError::CommandFailed { code: 2, .. }));
    }

    // ---- M4-T1: tasks ----

    const TASKS_JSON: &str =
        r#"[{"name":"build","depends":["lint"],"run":["echo build"]},{"name":"test:unit"}]"#;

    #[test]
    fn tasks_argv_scoped_and_parsed() {
        let (m, seen, _) = mise_rec_stream(TASKS_JSON);
        let ts = m.tasks(Some(Path::new("/proj"))).unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["-C", "/proj", "tasks", "ls", "--json"]
        );
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].name, "build");
        assert_eq!(ts[0].depends, vec!["lint"]);
        assert_eq!(ts[1].name, "test:unit"); // colon name survives verbatim
    }

    #[test]
    fn tasks_argv_unscoped() {
        let (m, seen, _) = mise_rec_stream("[]");
        m.tasks(None).unwrap();
        assert_eq!(seen.lock().unwrap()[0], vec!["tasks", "ls", "--json"]);
    }

    #[test]
    fn run_task_stream_argv_keeps_task_name_verbatim() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.run_task_stream("test:unit", Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "run", "test:unit"]
        );
    }

    #[test]
    fn watch_task_stream_argv_scoped() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.watch_task_stream("lint", Some(Path::new("/proj")));
        assert_eq!(
            stream.lock().unwrap()[0],
            vec!["-C", "/proj", "watch", "lint"]
        );
    }

    #[test]
    fn run_and_watch_argv_without_project() {
        let (m, _, stream) = mise_rec_stream("");
        let _ = m.run_task_stream("lint", None);
        let _ = m.watch_task_stream("lint", None);
        assert_eq!(stream.lock().unwrap()[0], vec!["run", "lint"]);
        assert_eq!(stream.lock().unwrap()[1], vec!["watch", "lint"]);
    }

    #[test]
    fn tasks_non_zero_exit_is_command_failed() {
        struct Failing;
        impl CommandRunner for Failing {
            fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
                Ok(CommandOutput {
                    code: 1,
                    stdout: String::new(),
                    stderr: "config not trusted".into(),
                })
            }
        }
        let m = Mise::new("mise".into(), Arc::new(Failing), Arc::new(NoopStreamer));
        let err = m.tasks(Some(Path::new("/proj"))).unwrap_err();
        assert!(matches!(err, CoreError::CommandFailed { code: 1, .. }));
    }
}
