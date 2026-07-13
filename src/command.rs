use std::ffi::OsStr;
use std::fmt;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, ExitStatus, Output, Stdio};
use crate::Error;
use crate::resolve::resolve;

pub struct DoubleEscapeValidator(pub Box<dyn Fn(&Path) -> bool + Send + Sync>);

impl fmt::Debug for DoubleEscapeValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DoubleEscapeValidator")
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
    /// when true, the command is always run through a shell
    /// (`cmd.exe /d /s /c` on Windows, `sh -c` on Unix).
    shell: bool,
    /// hidden option that forces shell wrapping even
    /// for files that would normally be executed directly.
    force_shell: bool,
    #[cfg(windows)]
    windows_hide: bool,
    #[cfg(windows)]
    double_escape_validator: Option<DoubleEscapeValidator>,
}

impl Command {

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
            shell: false,
            force_shell: false,
            #[cfg(windows)]
            windows_hide: false,
            #[cfg(windows)]
            double_escape_validator: None,
        }
    }

    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.args.push(arg.as_ref().to_string_lossy().into_owned());
        self
    }

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

    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.cwd = Some(dir.as_ref().to_path_buf());
        self
    }

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

    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Self {
        self.env_vars
            .push((key.as_ref().to_string_lossy().into_owned(), None));
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stdin = Some(cfg.into());
        self
    }
    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stdout = Some(cfg.into());
        self
    }

    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.stderr = Some(cfg.into());
        self
    }
    pub fn shell(&mut self, enable: bool) -> &mut Self {
        self.shell = enable;
        self
    }
    pub fn force_shell(&mut self, enable: bool) -> &mut Self {
        self.force_shell = enable;
        self
    }

    /// On Windows, prevents the child process from creating a console window.
    /// This sets `CREATE_NO_WINDOW` (0x08000000) in the process creation flags.
    /// On non-Windows platforms this is a no-op.
    #[cfg(windows)]
    pub fn windows_hide(&mut self, hide: bool) -> &mut Self {
        self.windows_hide = hide;
        self
    }

    /// Configures a custom callback to check whether a resolved file path should
    /// trigger double caret escaping for cmd.exe shell execution on Windows.
    #[cfg(windows)]
    pub fn double_escape_validator<F>(&mut self, validator: F) -> &mut Self
    where
        F: Fn(&Path) -> bool + Send + Sync + 'static,
    {
        self.double_escape_validator = Some(DoubleEscapeValidator(Box::new(validator)));
        self
    }

    /// Spawns the process and returns a [`Child`] handle for streaming I/O.
    /// Inherits stdin/stdout/stderr from the parent process by default.
    /// Use `stdin()`, `stdout()`, `stderr()` to pipe them.
    pub fn spawn(&mut self) -> Result<Child, Error> {
        self._spawn()
    }

    /// Executes the command, waits for it to finish, and collects stdout/stderr.
    /// Automatically pipes stdin/stdout/stderr.
    pub fn output(&mut self) -> Result<Output, Error> {
        self.stdin.get_or_insert(Stdio::piped());
        self.stdout.get_or_insert(Stdio::piped());
        self.stderr.get_or_insert(Stdio::piped());
        let child = self._spawn()?;
        let output = child.wait_with_output()?;
        Ok(output)
    }

    pub fn status(&mut self) -> Result<ExitStatus, Error> {
        let mut child = self._spawn()?;
        child.wait().map_err(Error::Io)
    }

    fn _spawn(&mut self) -> Result<Child, Error> {
        if self.program.is_empty() {
            return Err(Error::EmptyCommand);
        }

        // Unconditional NUL byte check (G3/G4)
        if self.program.contains('\0') {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Command path cannot contain null bytes"
            )));
        }
        for arg in &self.args {
            if arg.contains('\0') {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Arguments cannot contain null bytes"
                )));
            }
        }
        for (k, v) in &self.env_vars {
            if k.contains('\0') {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Environment variable keys cannot contain null bytes"
                )));
            }
            if let Some(val) = v {
                if val.contains('\0') {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Environment variable values cannot contain null bytes"
                    )));
                }
            }
        }

        if self.shell {
            return self.spawn_via_shell();
        }

        let resolved = resolve(
            &self.program,
            &self.args,
            self.force_shell,
            #[cfg(windows)]
            self.double_escape_validator.as_ref().map(|v| &*v.0 as &dyn Fn(&Path) -> bool),
            #[cfg(not(windows))]
            None,
            &self.cwd,
            &self.env_vars,
            self.env_clear,
        )?;

        let mut cmd = StdCommand::new(&resolved.program);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if resolved.verbatim {
                // IMPORTANT: `raw_arg` does not add quoting of its own, so we
                // must pass the *entire* command tail as a single `raw_arg`.
                // Passing the tokens individually would either leave the `/c`
                // value unquoted (cmd's `/s` then does not strip the surrounding
                // quotes) or, when pre-quoted, get double-quoted by `raw_arg`.
                // A single `raw_arg` preserves exactly one level of quoting,
                // matching node-cross-spawn's behaviour.
                cmd.raw_arg(resolved.args.join(" "));
            } else {
                cmd.args(&resolved.args);
            }
        }
        #[cfg(not(windows))]
        {
            cmd.args(&resolved.args);
        }

        self.apply_to(&mut cmd);

        cmd.spawn().map_err(|e| map_io_err(e, &self.program))
    }

    #[cfg(windows)]
    fn spawn_via_shell(&mut self) -> Result<Child, Error> {
        // Validate inputs for control characters to prevent command injection (CVE-2024-24576 / BatBadBut)
        if self.program.contains('\r') || self.program.contains('\n') {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Command path cannot contain carriage returns or newlines on Windows"
            )));
        }
        for arg in &self.args {
            if arg.contains('\r') || arg.contains('\n') {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Arguments cannot contain carriage returns or newlines when executing via shell on Windows"
                )));
            }
        }
        for (k, v) in &self.env_vars {
            if k.contains('\r') || k.contains('\n') {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Environment variable keys cannot contain carriage returns or newlines on Windows"
                )));
            }
            if let Some(val) = v {
                if val.contains('\r') || val.contains('\n') {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Environment variable values cannot contain carriage returns or newlines on Windows"
                    )));
                }
            }
        }

        let joined = std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let shell_cmd = format!("\"{}\"", joined);
        let mut cmd = StdCommand::new(
            std::env::var("comspec").unwrap_or_else(|_| "cmd.exe".to_string()),
        );
        use std::os::windows::process::CommandExt;
        cmd.raw_arg("/d").raw_arg("/s").raw_arg("/c").raw_arg(&shell_cmd);
        self.apply_to(&mut cmd);
        cmd.spawn().map_err(|e| map_io_err(e, &self.program))
    }

    #[cfg(not(windows))]
    fn spawn_via_shell(&mut self) -> Result<Child, Error> {
        let joined = std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let mut cmd = StdCommand::new("sh");
        cmd.arg("-c").arg(&joined);
        self.apply_to(&mut cmd);
        cmd.spawn().map_err(|e| map_io_err(e, &self.program))
    }

    fn apply_to(&mut self, cmd: &mut StdCommand) {
        if let Some(ref cwd) = self.cwd {
            cmd.current_dir(cwd);
        }
        if self.env_clear {
            cmd.env_clear();
        }
        for (k, v) in &self.env_vars {
            match v {
                Some(val) => {
                    cmd.env(k, val);
                }
                None => {
                    cmd.env_remove(k);
                }
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
    }
}

fn map_io_err(e: IoError, program: &str) -> Error {
    if e.kind() == ErrorKind::NotFound {
        Error::NotFound(program.to_string())
    } else {
        Error::Io(e)
    }
}
