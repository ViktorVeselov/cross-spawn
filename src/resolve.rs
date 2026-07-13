use std::path::PathBuf;
use crate::Error;

#[cfg(windows)]
use crate::windows::{
    escape_argument, escape_command, get_effective_path, get_pathext, is_cmd_builtin,
    is_executable, normalize_posix, read_shebang, resolve_command_file,
};

pub struct Resolved {
    pub program: String,
    pub args: Vec<String>,
    /// when true, the arguments are already escaped and must be passed
    /// verbatim (raw_arg on Windows).
    pub verbatim: bool,
}

#[cfg(windows)]
pub(crate) fn resolve(
    program: &str,
    args: &[String],
    force_shell: bool,
    double_escape_validator: Option<&dyn Fn(&std::path::Path) -> bool>,
    cwd: &Option<PathBuf>,
    env_vars: &[(String, Option<String>)],
    env_clear: bool,
) -> Result<Resolved, Error> {
    // Validate inputs for control characters to prevent command injection (CVE-2024-24576 / BatBadBut)
    if program.contains('\r') || program.contains('\n') {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Command path cannot contain carriage returns or newlines on Windows"
        )));
    }
    for arg in args {
        if arg.contains('\r') || arg.contains('\n') {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Arguments cannot contain carriage returns or newlines when executing via shell on Windows"
            )));
        }
    }
    for (k, v) in env_vars {
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

    let cwd = cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let pathext = get_pathext();
    let env_path = get_effective_path(env_vars, env_clear);
    let file = resolve_command_file(program, &cwd, &pathext, &env_path, true)
        .or_else(|| resolve_command_file(program, &cwd, &pathext, &env_path, false));

    let (command_file, command, new_args) = match file {
        Some(f) => {

            if let Some(shebang) = read_shebang(&f) {
                let mut a = vec![f.to_string_lossy().into_owned()];
                a.extend(args.iter().cloned());
                let interp = resolve_command_file(&shebang, &cwd, &pathext, &env_path, true)
                    .or_else(|| {
                        resolve_command_file(&shebang, &cwd, &pathext, &env_path, false)
                    });
                let cmd = interp
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| shebang.clone());
                (interp, cmd, a)
            } else {
                (Some(f), program.to_string(), args.to_vec())
            }
        }
        None => {
            // ENOENT pre-flight. If the command file is not found and it
            // is not a CMD builtin, fail immediately. CMD builtins (e.g. `echo`,
            // `dir`) have no backing file, so route them through the normal
            // shell-wrapping path below (`/d /s /c "..."` with proper escaping),
            // exactly like node-cross-spawn does.
            if !is_cmd_builtin(program) {
                return Err(Error::NotFound(program.to_string()));
            }
            (None, program.to_string(), args.to_vec())
        }
    };

    let needs_shell = match &command_file {
        Some(f) => !is_executable(f),
        None => true,
    };

    if force_shell || needs_shell {
        let normalized = normalize_posix(&command);
        let escaped_cmd = escape_command(&normalized);
        let needs_double = match (&command_file, double_escape_validator) {
            (Some(f), Some(validator)) => validator(f),
            _ => false,
        };
        let escaped_args: Vec<String> = new_args
            .iter()
            .map(|a| escape_argument(a, needs_double))
            .collect();
        let shell_cmd = std::iter::once(escaped_cmd)
            .chain(escaped_args)
            .collect::<Vec<_>>()
            .join(" ");
        // Wrap the whole shell command in quotes. This mirrors original node-cross-spawn
        // (`'"' + shellCommand + '"'`) and is required so that cmd.exe's `/s`
        // switch strips the surrounding quotes and processes the interior
        // `^`-escaping. `raw_arg` passes it verbatim (it does not re-quote an
        // argument that already begins/ends with a quote), so this is the
        // single, correct level of quoting.
        let final_args = vec![
            "/d".to_string(),
            "/s".to_string(),
            "/c".to_string(),
            format!("\"{}\"", shell_cmd),
        ];
        return Ok(Resolved {
            program: "cmd.exe".to_string(),
            args: final_args,
            verbatim: true,
        });
    }

    Ok(Resolved {
        program: command,
        args: new_args,
        verbatim: false,
    })
}

#[cfg(not(windows))]
pub(crate) fn resolve(
    program: &str,
    args: &[String],
    _force_shell: bool,
    _double_escape_validator: Option<&dyn Fn(&std::path::Path) -> bool>,
    _cwd: &Option<PathBuf>,
    _env_vars: &[(String, Option<String>)],
    _env_clear: bool,
) -> Result<Resolved, Error> {
    Ok(Resolved {
        program: program.to_string(),
        args: args.to_vec(),
        verbatim: false,
    })
}
