use crate::error::CoreError;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Stdout(String),
    Stderr(String),
    Exited(i32),
}

pub trait StreamingRunner: Send + Sync {
    fn spawn(&self, req: CommandRequest) -> Result<Receiver<StreamEvent>, CoreError>;
}

pub struct ProcessStreamer;

impl StreamingRunner for ProcessStreamer {
    fn spawn(&self, req: CommandRequest) -> Result<Receiver<StreamEvent>, CoreError> {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut cmd = Command::new(&req.program);
            cmd.args(&req.args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(cwd) = &req.cwd {
                cmd.current_dir(cwd);
            }
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(StreamEvent::Stderr(e.to_string()));
                    let _ = tx.send(StreamEvent::Exited(-1));
                    return;
                }
            };
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let tx_out = tx.clone();
            let h_out = stdout.map(|s| {
                std::thread::spawn(move || {
                    for line in BufReader::new(s).lines().map_while(Result::ok) {
                        let _ = tx_out.send(StreamEvent::Stdout(line));
                    }
                })
            });
            let tx_err = tx.clone();
            let h_err = stderr.map(|s| {
                std::thread::spawn(move || {
                    for line in BufReader::new(s).lines().map_while(Result::ok) {
                        let _ = tx_err.send(StreamEvent::Stderr(line));
                    }
                })
            });
            let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
            if let Some(h) = h_out {
                let _ = h.join();
            }
            if let Some(h) = h_err {
                let _ = h.join();
            }
            let _ = tx.send(StreamEvent::Exited(code));
        });
        Ok(rx)
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
