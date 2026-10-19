use crate::error::CoreError;
use crate::registry::parse_registry;
use crate::remote::parse_remote_versions;
use crate::runner::{CommandOutput, CommandRequest, CommandRunner, StreamEvent, StreamingRunner};
use crate::tools::InstalledSet;
use crate::types::{RemoteVersion, ToolEntry};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

pub struct Mise {
    program: PathBuf,
    runner: Arc<dyn CommandRunner>,
    streamer: Arc<dyn StreamingRunner>,
}

impl Mise {
    pub fn new(program: PathBuf, runner: Arc<dyn CommandRunner>, streamer: Arc<dyn StreamingRunner>) -> Self {
        Self { program, runner, streamer }
    }

    pub fn version(&self) -> Result<String, CoreError> {
        let out = self.run(&[], None)?;
        Ok(out.stdout.split_whitespace().next().unwrap_or_default().to_string())
    }

    pub fn registry(&self) -> Result<Vec<ToolEntry>, CoreError> {
        let out = self.run(&["registry", "--json"], None)?;
        parse_registry(&out.stdout)
    }

    pub fn installed(&self, project_root: Option<&Path>) -> Result<InstalledSet, CoreError> {
        let out = self.run(&["ls", "--json"], project_root)?;
        InstalledSet::parse(&out.stdout)
    }

    pub fn remote_versions(&self, tool: &str, project_root: Option<&Path>) -> Result<Vec<RemoteVersion>, CoreError> {
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
        self.streamer.spawn(CommandRequest { program: self.program.clone(), args, cwd: None })
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
            return Err(CoreError::CommandFailed { code: out.code, stderr: out.stderr });
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
            Ok(CommandOutput { code: 0, stdout: self.stdout.clone(), stderr: String::new() })
        }
    }

    struct NoopStreamer;
    impl StreamingRunner for NoopStreamer {
        fn spawn(&self, _req: CommandRequest) -> Result<std::sync::mpsc::Receiver<StreamEvent>, CoreError> {
            Ok(std::sync::mpsc::channel().1)
        }
    }

    fn mise(stdout: &str) -> (Mise, Arc<Mutex<Vec<Vec<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = Arc::new(RecordingRunner { seen: seen.clone(), stdout: stdout.into() });
        (Mise::new("mise".into(), runner, Arc::new(NoopStreamer)), seen)
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
        let (m, _) = mise("2026.10.13 macos-arm64 (2026-10-17)\n");
        assert_eq!(m.version().unwrap(), "2026.10.13");
    }

    #[test]
    fn tool_name_with_backend_prefix_is_verbatim() {
        let (m, seen) = mise("[]");
        let _ = m.remote_versions("npm:prettier", None);
        assert_eq!(seen.lock().unwrap()[0], vec!["ls-remote", "npm:prettier", "--json"]);
    }

    #[test]
    fn non_zero_exit_becomes_command_failed() {
        struct Failing;
        impl CommandRunner for Failing {
            fn run(&self, _req: &CommandRequest) -> Result<CommandOutput, CoreError> {
                Ok(CommandOutput { code: 2, stdout: String::new(), stderr: "boom".into() })
            }
        }
        let m = Mise::new("mise".into(), Arc::new(Failing), Arc::new(NoopStreamer));
        let err = m.registry().unwrap_err();
        assert!(matches!(err, CoreError::CommandFailed { code: 2, .. }));
    }
}
