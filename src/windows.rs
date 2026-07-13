use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(windows)]
fn is_meta(c: char) -> bool {
    matches!(
        c,
        '(' | ')' | ']' | '[' | '%' | '!' | '^' | '"' | '<' | '>' | '&' | '|' | ';' | ',' | ' '
            | '*' | '?' | '`'
    )
}

#[cfg(windows)]
fn escape_meta(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_meta(c) {
            out.push('^');
        }
        out.push(c);
    }
    out
}

/// Escape a command (program) for cmd.exe by prefixing every
/// metacharacter with `^`.
#[cfg(windows)]
pub fn escape_command(cmd: &str) -> String {
    escape_meta(cmd)
}

/// Escape an argument for cmd.exe using the qntm backslash-doubling
/// algorithm, quote the result, and prefix metacharacters with `^`. When
/// `double_escape` is set (cmd-shims), metacharacters are escaped a second time.
#[cfg(windows)]
pub fn escape_argument(arg: &str, double_escape: bool) -> String {
    let bytes = arg.as_bytes();
    let n = bytes.len();
    let mut inner = String::new();
    let mut idx = 0;
    while idx < n {
        if bytes[idx] == b'\\' {
            let start = idx;
            while idx < n && bytes[idx] == b'\\' {
                idx += 1;
            }
            let count = idx - start;
            if idx < n && bytes[idx] == b'"' {
                // Lazy capture: only the last backslash is "doubled"; the rest
                // stay verbatim. The matched quote is rewritten as `\"`
                // (backslash + quote), mirroring node's `$1$1\"` replacement.
                for _ in 0..(count.saturating_sub(1)) {
                    inner.push('\\');
                }
                inner.push_str("\\\\\\");
                inner.push('"');
                idx += 1;
            } else if idx >= n {
                // Trailing backslashes at end of string: double them.
                for _ in 0..(count.saturating_sub(1)) {
                    inner.push('\\');
                }
                inner.push_str("\\\\");
            } else {
                for _ in 0..count {
                    inner.push('\\');
                }
            }
        } else if bytes[idx] == b'"' {
            inner.push_str("\\\"");
            idx += 1;
        } else {
            inner.push(bytes[idx] as char);
            idx += 1;
        }
    }

    let mut result = format!("\"{}\"", inner);
    result = escape_meta(&result);
    if double_escape {
        result = escape_meta(&result);
    }
    result
}

/// Read the shebang interpreter from the first 150 bytes of a file.
/// Handles `/usr/bin/env <interp>` by returning `<interp>` (assumption A2).
#[cfg(windows)]
pub fn read_shebang(path: &Path) -> Option<String> {
    let mut buf = [0u8; 150];
    let mut file = File::open(path).ok()?;
    let n = file.read(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    let line = text.lines().next()?;
    if !line.starts_with("#!") {
        return None;
    }
    let rest = line[2..].trim();
    let mut parts = rest.split_whitespace();
    let first = parts.next()?;
    if first.ends_with("/env") || first.ends_with("\\env") {
        parts.next().filter(|s| !s.is_empty()).map(|s| s.to_string())
    } else {
        let base = first.rsplit('/').next().unwrap_or(first);
        let base = base.rsplit('\\').next().unwrap_or(base);
        Some(base.to_string())
    }
}

/// Whether the resolved path is a directly-executable Windows image.
#[cfg(windows)]
pub fn is_executable(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase();
    lower.ends_with(".exe") || lower.ends_with(".com")
}

#[cfg(windows)]
pub fn get_pathext() -> Vec<String> {
    std::env::var("PATHEXT")
        .map(|v| v.split(';').map(|s| s.to_string()).collect())
        .unwrap_or_else(|_| {
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC"
                .split(';')
                .map(|s| s.to_string())
                .collect()
        })
}

#[cfg(windows)]
pub fn get_effective_path(env_vars: &[(String, Option<String>)], env_clear: bool) -> String {
    let mut map: HashMap<String, String> = if env_clear {
        HashMap::new()
    } else {
        std::env::vars().collect()
    };
    for (k, v) in env_vars {
        match v {
            Some(val) => {
                map.insert(k.clone(), val.clone());
            }
            None => {
                map.remove(k);
            }
        }
    }
    let key = map.keys().find(|k| k.eq_ignore_ascii_case("Path")).cloned();
    key.and_then(|k| map.get(&k).cloned()).unwrap_or_default()
}

/// Resolve a command file by searching `cwd` first, then each directory
/// in the provided PATH env, using PATHEXT extensions when `with_ext` is true.
#[cfg(windows)]
pub fn resolve_command_file(
    program: &str,
    cwd: &Path,
    pathext: &[String],
    env_path: &str,
    with_ext: bool,
) -> Option<PathBuf> {
    let norm = program.replace('/', "\\");
    let has_sep = norm.contains('\\') || Path::new(&norm).is_absolute();

    if has_sep {
        let base = if Path::new(&norm).is_absolute() {
            PathBuf::from(&norm)
        } else {
            cwd.join(&norm)
        };
        if with_ext {
            for ext in pathext {
                let e = ext.trim_start_matches('.');
                let candidate = base.with_extension(e);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        if base.is_file() {
            return Some(base);
        }
        return None;
    }

    // Bare command: search cwd, then PATH directories.
    let search = |dir: &Path| -> Option<PathBuf> {
        if with_ext {
            for ext in pathext {
                let candidate = dir.join(format!("{}{}", program, ext));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            None
        } else {
            let candidate = dir.join(program);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        }
    };

    if let Some(f) = search(cwd) {
        return Some(f);
    }
    for dir in std::env::split_paths(env_path) {
        if let Some(f) = search(&dir) {
            return Some(f);
        }
    }
    None
}

#[cfg(windows)]
pub fn normalize_posix(s: &str) -> String {
    s.replace('/', "\\")
}

#[cfg(windows)]
pub fn is_cmd_builtin(program: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn test_escape_matches_reference() {
        assert_eq!(escape_command("foo"), "foo");
        assert_eq!(escape_argument("foo", false), "^\"foo^\"");
        assert_eq!(escape_argument("foo", true), "^^^\"foo^^^\"");
        assert_eq!(escape_command(""), "");
        assert_eq!(escape_argument("", false), "^\"^\"");
        assert_eq!(escape_argument("", true), "^^^\"^^^\"");
        assert_eq!(escape_command("bar"), "bar");
        assert_eq!(escape_argument("bar", false), "^\"bar^\"");
        assert_eq!(escape_argument("bar", true), "^^^\"bar^^^\"");
        assert_eq!(escape_command("()"), "^(^)");
        assert_eq!(escape_argument("()", false), "^\"^(^)^\"");
        assert_eq!(escape_argument("()", true), "^^^\"^^^(^^^)^^^\"");
        assert_eq!(escape_command("[]"), "^[^]");
        assert_eq!(escape_argument("[]", false), "^\"^[^]^\"");
        assert_eq!(escape_argument("[]", true), "^^^\"^^^[^^^]^^^\"");
        assert_eq!(escape_command("%!"), "^%^!");
        assert_eq!(escape_argument("%!", false), "^\"^%^!^\"");
        assert_eq!(escape_argument("%!", true), "^^^\"^^^%^^^!^^^\"");
        assert_eq!(escape_command("^<"), "^^^<");
        assert_eq!(escape_argument("^<", false), "^\"^^^<^\"");
        assert_eq!(escape_argument("^<", true), "^^^\"^^^^^^^<^^^\"");
        assert_eq!(escape_command(">&"), "^>^&");
        assert_eq!(escape_argument(">&", false), "^\"^>^&^\"");
        assert_eq!(escape_argument(">&", true), "^^^\"^^^>^^^&^^^\"");
        assert_eq!(escape_command("|;"), "^|^;");
        assert_eq!(escape_argument("|;", false), "^\"^|^;^\"");
        assert_eq!(escape_argument("|;", true), "^^^\"^^^|^^^;^^^\"");
        assert_eq!(escape_command(", "), "^,^ ");
        assert_eq!(escape_argument(", ", false), "^\"^,^ ^\"");
        assert_eq!(escape_argument(", ", true), "^^^\"^^^,^^^ ^^^\"");
        assert_eq!(escape_command("!="), "^!=");
        assert_eq!(escape_argument("!=", false), "^\"^!=^\"");
        assert_eq!(escape_argument("!=", true), "^^^\"^^^!=^^^\"");
        assert_eq!(escape_command("\\*"), "\\^*");
        assert_eq!(escape_argument("\\*", false), "^\"\\^*^\"");
        assert_eq!(escape_argument("\\*", true), "^^^\"\\^^^*^^^\"");
        assert_eq!(escape_command("\"f\""), "^\"f^\"");
        assert_eq!(escape_argument("\"f\"", false), "^\"\\^\"f\\^\"^\"");
        assert_eq!(escape_argument("\"f\"", true), "^^^\"\\^^^\"f\\^^^\"^^^\"");
        assert_eq!(escape_command("?."), "^?.");
        assert_eq!(escape_argument("?.", false), "^\"^?.^\"");
        assert_eq!(escape_argument("?.", true), "^^^\"^^^?.^^^\"");
        assert_eq!(escape_command("=`"), "=^`");
        assert_eq!(escape_argument("=`", false), "^\"=^`^\"");
        assert_eq!(escape_argument("=`", true), "^^^\"=^^^`^^^\"");
        assert_eq!(escape_command("'"), "'");
        assert_eq!(escape_argument("'", false), "^\"'^\"");
        assert_eq!(escape_argument("'", true), "^^^\"'^^^\"");
        assert_eq!(escape_command("\\\""), "\\^\"");
        assert_eq!(escape_argument("\\\"", false), "^\"\\\\\\^\"^\"");
        assert_eq!(escape_argument("\\\"", true), "^^^\"\\\\\\^^^\"^^^\"");
        assert_eq!(escape_command("bar\\"), "bar\\");
        assert_eq!(escape_argument("bar\\", false), "^\"bar\\\\^\"");
        assert_eq!(escape_argument("bar\\", true), "^^^\"bar\\\\^^^\"");
        assert_eq!(escape_command("\"(foo|bar>baz)\""), "^\"^(foo^|bar^>baz^)^\"");
        assert_eq!(escape_argument("\"(foo|bar>baz)\"", false), "^\"\\^\"^(foo^|bar^>baz^)\\^\"^\"");
        assert_eq!(escape_argument("\"(foo|bar>baz)\"", true), "^^^\"\\^^^\"^^^(foo^^^|bar^^^>baz^^^)\\^^^\"^^^\"");
        assert_eq!(escape_command("\"(foo|bar>baz|foz)\""), "^\"^(foo^|bar^>baz^|foz^)^\"");
        assert_eq!(escape_argument("\"(foo|bar>baz|foz)\"", false), "^\"\\^\"^(foo^|bar^>baz^|foz^)\\^\"^\"");
        assert_eq!(escape_argument("\"(foo|bar>baz|foz)\"", true), "^^^\"\\^^^\"^^^(foo^^^|bar^^^>baz^^^|foz^^^)\\^^^\"^^^\"");
        assert_eq!(escape_command("pre_()%!^&;, "), "pre_^(^)^%^!^^^&^;^,^ ");
        assert_eq!(escape_argument("pre_()%!^&;, ", false), "^\"pre_^(^)^%^!^^^&^;^,^ ^\"");
        assert_eq!(escape_argument("pre_()%!^&;, ", true), "^^^\"pre_^^^(^^^)^^^%^^^!^^^^^^^&^^^;^^^,^^^ ^^^\"");
        assert_eq!(escape_command("echo %RANDOM%"), "echo^ ^%RANDOM^%");
        assert_eq!(escape_argument("echo %RANDOM%", false), "^\"echo^ ^%RANDOM^%^\"");
        assert_eq!(escape_argument("echo %RANDOM%", true), "^^^\"echo^^^ ^^^%RANDOM^^^%^^^\"");
        assert_eq!(escape_command("hello && echo there"), "hello^ ^&^&^ echo^ there");
        assert_eq!(escape_argument("hello && echo there", false), "^\"hello^ ^&^&^ echo^ there^\"");
        assert_eq!(escape_argument("hello && echo there", true), "^^^\"hello^^^ ^^^&^^^&^^^ echo^^^ there^^^\"");
        assert_eq!(escape_command("()%!^&;, "), "^(^)^%^!^^^&^;^,^ ");
        assert_eq!(escape_argument("()%!^&;, ", false), "^\"^(^)^%^!^^^&^;^,^ ^\"");
        assert_eq!(escape_argument("()%!^&;, ", true), "^^^\"^^^(^^^)^^^%^^^!^^^^^^^&^^^;^^^,^^^ ^^^\"");
    }

    #[cfg(windows)]
    fn setup_unit_fixtures() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/unit_fixtures");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shebang"), "#!/usr/bin/env test_helper\nshebang works!").unwrap();
        std::fs::write(dir.join("say-foo"), "#!/usr/bin/env bash\n").unwrap();
        std::fs::write(dir.join("shebang-enoent"), "#!/usr/bin/env somecommandthatwillneverexist\n").unwrap();
        std::fs::write(dir.join("say-foo.bat"), "@echo foo\n").unwrap();
        dir
    }

    #[cfg(windows)]
    #[test]
    fn test_read_shebang() {
        let dir = setup_unit_fixtures();
        assert_eq!(
            read_shebang(&dir.join("shebang")),
            Some("test_helper".to_string())
        );
        assert_eq!(
            read_shebang(&dir.join("say-foo")),
            Some("bash".to_string())
        );
        assert_eq!(
            read_shebang(&dir.join("shebang-enoent")),
            Some("somecommandthatwillneverexist".to_string())
        );
        assert_eq!(read_shebang(&dir.join("say-foo.bat")), None);
    }

    #[cfg(windows)]
    #[test]
    fn test_resolve_pathext() {
        let dir = setup_unit_fixtures();
        let pathext = get_pathext();
        let found = resolve_command_file("say-foo", &dir, &pathext, "", true);
        assert!(found
            .map(|p| p.to_string_lossy().to_lowercase().ends_with("say-foo.bat"))
            .unwrap_or(false));
        let found = resolve_command_file("shebang", &dir, &pathext, "", true)
            .or_else(|| resolve_command_file("shebang", &dir, &pathext, "", false));
        assert!(found
            .map(|p| p.to_string_lossy().to_lowercase().ends_with("shebang"))
            .unwrap_or(false));
    }
}
