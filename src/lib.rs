use std::io;
use std::process::{Command as StdCommand, Stdio};
use thiserror::Error;

pub mod command;
pub mod resolve;
#[cfg(windows)]
pub mod windows;

pub use command::Command;

#[derive(Error, Debug)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("program not found: {0}")]
    NotFound(String),
    #[error("empty command")]
    EmptyCommand,
    #[error("invalid pid: {0}")]
    InvalidPid(u32),
}

impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(e) => e,
            Error::NotFound(msg) => io::Error::new(io::ErrorKind::NotFound, msg),
            Error::EmptyCommand => io::Error::new(io::ErrorKind::InvalidInput, "empty command"),
            Error::InvalidPid(_) => io::Error::new(io::ErrorKind::InvalidInput, e.to_string()),
        }
    }
}

/// Kills a process by PID cross-platform.
///
/// On Windows, uses `taskkill /T /F` to terminate the process tree.
/// On Unix, sends `SIGTERM`.
pub fn kill(pid: u32) -> Result<(), Error> {
    if pid == 0 {
        return Err(Error::InvalidPid(pid));
    }

    #[cfg(target_os = "windows")]
    {
        let status = StdCommand::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(Error::Io)?;
        if !status.success() {
            return Err(Error::NotFound(format!("failed to kill pid {}", pid)));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let status = StdCommand::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(Error::Io)?;
        if !status.success() {
            return Err(Error::NotFound(format!("failed to kill pid {}", pid)));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_echo() {
        let mut cmd = Command::new("echo");
        let output = cmd.args(["hello"]).output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello"));
    }

    #[test]
    fn test_spawn_cmd() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "echo", "test"]);
        let output = cmd.output().unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn test_status() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "exit", "0"]);
        let status = cmd.status().unwrap();
        assert!(status.success());
    }

    #[test]
    fn test_not_found() {
        let mut cmd = Command::new("nonexistent_command_xyz");
        let result = cmd.spawn();
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NotFound(_) => {}
            other => panic!("expected NotFound, got: {}", other),
        }
    }

    #[test]
    fn test_empty_command() {
        let mut cmd = Command::new("");
        let result = cmd.spawn();
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::EmptyCommand => {}
            other => panic!("expected EmptyCommand, got: {}", other),
        }
    }

    #[test]
    fn test_env() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "set", "MY_TEST_VAR"]);
        cmd.env("MY_TEST_VAR", "hello");
        let output = cmd.output().unwrap();
        assert!(output.status.success());
        let out = String::from_utf8_lossy(&output.stdout);
        assert!(out.contains("hello"));
    }

    #[test]
    fn test_cwd() {
        let tmp = std::env::temp_dir();
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "echo", "%CD%"]);
        cmd.current_dir(&tmp);
        let output = cmd.output().unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn test_kill_invalid_pid() {
        let result = kill(0);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::InvalidPid(0) => {}
            other => panic!("expected InvalidPid, got: {}", other),
        }
    }
}
