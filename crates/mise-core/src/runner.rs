use crate::error::CoreError;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError>;
}

pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&self, req: &CommandRequest) -> Result<CommandOutput, CoreError> {
        let mut cmd = Command::new(&req.program);
        cmd.args(&req.args);
        if let Some(cwd) = &req.cwd {
            cmd.current_dir(cwd);
        }
        let out = cmd.output()?;
        Ok(CommandOutput {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_keeps_args_verbatim() {
        let req = CommandRequest {
            program: PathBuf::from("/bin/echo"),
            args: vec!["-C".into(), "/tmp/a b".into(), "ls".into(), "--json".into()],
            cwd: None,
        };
        assert_eq!(req.args[1], "/tmp/a b");
    }
}
