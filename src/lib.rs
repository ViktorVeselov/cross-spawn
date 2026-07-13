use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, ExitStatus, Output, Stdio};
use thiserror::Error;

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

#[derive(Debug)]
pub struct Command {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env_clear: bool,
    env_vars: Vec<(String, Option<String>)>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    #[cfg(windows)]
    windows_hide: bool,
}

impl Command {
    /// Creates a new Command with the given program.
    ///
    /// On Windows, the program is resolved using PATHEXT and CMD built-in detection.
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        Command {
            program: program.as_ref().to_string_lossy().into_owned(),
            args: Vec::new(),
            cwd: None,
            env_clear: false,
            env_vars: Vec::new(),
            stdin: None,
            stdout: None,
            stderr: None,
            #[cfg(windows)]
            windows_hide: false,
        }
    }

    /// Adds a single argument.
    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.args.push(arg.as_ref().to_string_lossy().into_owned());
        self
    }

    /// Adds multiple arguments.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    /// Sets the working directory.
    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.cwd = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Sets an environment variable.
    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env_vars.push((
            key.as_ref().to_string_lossy().into_owned(),
            Some(val.as_ref().to_string_lossy().into_owned()),
        ));
        self
    }

    /// Removes an environment variable.
    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Self {
        self.env_vars
            .push((key.as_ref().to_string_lossy().into_owned(), None));
        self
    }

    /// Clears all environment variables.
    pub fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self
    }

    /// Sets stdin to a stdio configuration.
    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stdin = Some(cfg.into());
        self
    }

    /// Sets stdout to a stdio configuration.
    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stdout = Some(cfg.into());
        self
    }

    /// Sets stderr to a stdio configuration.
    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stderr = Some(cfg.into());
        self
    }

    /// On Windows, prevents the child process from creating a console window.
    ///
    /// This sets `CREATE_NO_WINDOW` (0x08000000) in the process creation flags.
    /// On non-Windows platforms this is a no-op.
    #[cfg(windows)]
    pub fn windows_hide(&mut self, hide: bool) -> &mut Self {
        self.windows_hide = hide;
        self
    }

    /// Spawns the process and returns a [`Child`] handle for streaming I/O.
    ///
    /// Inherits stdin/stdout/stderr from the parent process by default.
    /// Use `stdin()`, `stdout()`, `stderr()` to pipe them.
    pub fn spawn(&mut self) -> Result<Child, Error> {
        self._spawn()
    }

    /// Executes the command, waits for it to finish, and collects stdout/stderr.
    ///
    /// Automatically pipes stdin/stdout/stderr.
    pub fn output(&mut self) -> Result<Output, Error> {
        self.stdin.get_or_insert(Stdio::piped());
        self.stdout.get_or_insert(Stdio::piped());
        self.stderr.get_or_insert(Stdio::piped());
        let child = self._spawn()?;
        let output = child.wait_with_output()?;
        Ok(output)
    }

    /// Executes the command and waits for it to finish, returning the exit status.
    pub fn status(&mut self) -> Result<ExitStatus, Error> {
        let mut child = self._spawn()?;
        child.wait().map_err(Error::Io)
    }

    fn _spawn(&mut self) -> Result<Child, Error> {
        if self.program.is_empty() {
            return Err(Error::EmptyCommand);
        }

        let resolved = resolve(&self.program, &self.args)?;
        let mut cmd = StdCommand::new(&resolved.program);
        cmd.args(&resolved.args);

        if let Some(ref cwd) = self.cwd {
            cmd.current_dir(cwd);
        }

        if self.env_clear {
            cmd.env_clear();
        }
        for (key, val) in &self.env_vars {
            if let Some(val) = val {
                cmd.env(key, val);
            } else {
                cmd.env_remove(key);
            }
        }

        if let Some(v) = self.stdin.take() {
            cmd.stdin(v);
        }
        if let Some(v) = self.stdout.take() {
            cmd.stdout(v);
        }
        if let Some(v) = self.stderr.take() {
            cmd.stderr(v);
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if self.windows_hide {
                cmd.creation_flags(0x08000000);
            }
        }

        cmd.spawn().map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Error::NotFound(self.program.clone())
            } else {
                Error::Io(e)
            }
        })
    }
}

// ─── Resolution ───────────────────────────────────────────────────────────────

struct Resolved {
    program: String,
    args: Vec<String>,
}

#[cfg(windows)]
fn resolve(program: &str, args: &[String]) -> Result<Resolved, Error> {
    // If the program has a path separator or extension, use it as-is
    if program.contains('/') || program.contains('\\') || program.contains('.') {
        return Ok(Resolved {
            program: program.to_string(),
            args: args.to_vec(),
        });
    }

    // Try PATHEXT resolution
    if let Some(found) = resolve_pathext(program) {
        return Ok(Resolved {
            program: found,
            args: args.to_vec(),
        });
    }

    // Check CMD built-ins
    if is_cmd_builtin(program) {
        let mut new_args = vec!["/C".to_string(), program.to_string()];
        new_args.extend_from_slice(args);
        return Ok(Resolved {
            program: "cmd.exe".to_string(),
            args: new_args,
        });
    }

    Err(Error::NotFound(format!(
        "command not found: {}",
        program
    )))
}

#[cfg(windows)]
fn resolve_pathext(program: &str) -> Option<String> {
    let pathext = std::env::var_os("PATHEXT")
        .unwrap_or_else(|| {
            OsStr::new(".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC").to_os_string()
        });

    // Search current directory first (matching CreateProcessW behavior)
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = search_dir(program, &cwd, &pathext) {
            return Some(found);
        }
    }

    // Search directories in PATH
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if let Some(found) = search_dir(program, &dir, &pathext) {
                return Some(found);
            }
        }
    }

    None
}

#[cfg(windows)]
fn search_dir(program: &str, dir: &Path, pathext: &OsStr) -> Option<String> {
    for ext in std::env::split_paths(pathext) {
        let full = dir.join(format!("{}{}", program, ext.to_string_lossy()));
        if full.is_file() {
            return Some(full.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(windows)]
fn is_cmd_builtin(program: &str) -> bool {
    // Commands that are built into cmd.exe and have no standalone .exe
    const BUILTINS: &[&str] = &[
        "assoc", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir",
        "echo", "endlocal", "erase", "exit", "for", "ftype", "goto", "if", "md", "mkdir",
        "mklink", "move", "path", "pause", "popd", "prompt", "pushd", "rd", "rem", "ren",
        "rename", "rmdir", "set", "setlocal", "shift", "start", "time", "title", "type", "ver",
        "verify", "vol",
    ];
    let lower = program.to_lowercase();
    BUILTINS.contains(&lower.as_str())
}

#[cfg(not(windows))]
fn resolve(program: &str, args: &[String]) -> Result<Resolved, Error> {
    Ok(Resolved {
        program: program.to_string(),
        args: args.to_vec(),
    })
}

// ─── Kill ─────────────────────────────────────────────────────────────────────

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

// ─── Tests ────────────────────────────────────────────────────────────────────

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
