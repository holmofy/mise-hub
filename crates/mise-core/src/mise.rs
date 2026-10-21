use crate::error::CoreError;
use crate::manage::{UsePreview, parse_use_dry_run};
use crate::outdated::{OutdatedTool, parse_outdated};
use crate::registry::parse_registry;
use crate::remote::parse_remote_versions;
use crate::runner::{CommandOutput, CommandRequest, CommandRunner, StreamEvent, StreamingRunner};
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

    pub fn version(&self) -> Result<String, CoreError> {
        let out = self.run(&["--version"], None)?;
        Ok(out
            .stdout
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string())
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

    /// `mise trust <dir>`; non-zero exit becomes `CommandFailed`.
    pub fn trust(&self, dir: &Path) -> Result<(), CoreError> {
        self.run(&["trust", &dir.to_string_lossy()], None)?;
        Ok(())
    }

    /// `-C <dir>` for project scope, `--global`-less empty for global scope
    /// (callers add the scope flag after the subcommand).
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
        let mut full = self.base_args(project_root);
        full.extend(args.iter().map(|s| s.to_string()));
        let out = self.runner.run(&CommandRequest {
            program: self.program.clone(),
            args: full,
            cwd: None,
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
}
