use cross_spawn::Command;
use cross_spawn::Error;
use std::path::PathBuf;

use std::fs;
use std::sync::Once;

static INIT_FIXTURES: Once = Once::new();

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test_fixtures");
    
    INIT_FIXTURES.call_once(|| {
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("node_modules/.bin")).unwrap();
        fs::write(dir.join("say-foo"), "#!/usr/bin/env test_helper\n").unwrap();
        fs::write(dir.join("say-foo.bat"), "@test_helper echo foo\n").unwrap();
        fs::write(dir.join("shebang"), "#!/usr/bin/env test_helper\nshebang works!").unwrap();
        fs::write(dir.join("shebang-enoent"), "#!/usr/bin/env somecommandthatwillneverexist\n").unwrap();
        fs::write(dir.join("%CD%"), "#!/usr/bin/env test_helper\n").unwrap();
        fs::write(dir.join("%CD%.bat"), "@test_helper echo special\n").unwrap();
        fs::write(dir.join("()%!^&;, "), "#!/usr/bin/env test_helper\n").unwrap();
        fs::write(dir.join("()%!^&;, .bat"), "@test_helper echo special\n").unwrap();
        fs::write(dir.join("node_modules/.bin/echo-cmd-shim.cmd"), "@test_helper echo %*\n").unwrap();
        fs::write(dir.join("whoami.cmd"), "@echo you sure are someone\n").unwrap();
        fs::write(dir.join("exit-1"), "#!/usr/bin/env test_helper\n").unwrap();
        fs::write(dir.join("exit-1.bat"), "@test_helper exit 1\n").unwrap();

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_file() {
                    let mut perms = fs::metadata(&path).unwrap().permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&path, perms).unwrap();
                }
            }
        }
    });

    dir
}

fn run_out(program: &str, args: &[&str]) -> Result<(String, i32), Error> {
    let mut c = Command::new(program);
    c.args(args);
    // Prepend test_helper binary directory to PATH so shebang resolution can find it
    let helper_dir = std::path::Path::new(env!("CARGO_BIN_EXE_test_helper"))
        .parent()
        .unwrap()
        .to_path_buf();
    let cur = std::env::var("PATH").unwrap_or_default();
    c.env("PATH", format!("{};{}", helper_dir.to_string_lossy(), cur));
    let o = c.output()?;
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    let code = o.status.code().unwrap_or(-1);
    Ok((s, code))
}

fn norm(s: &str) -> String {
    s.replace('\r', "")
}

#[test]
fn pathext_resolution() {
    // Bare `say-foo` resolves to `say-foo.bat` on Windows, `say-foo` (bash) on Unix.
    let (out, _) = run_out(
        fixtures().join("say-foo").to_str().unwrap(),
        &[],
    )
    .expect("spawn");
    assert_eq!(norm(out.trim()), "foo");
}

#[cfg(windows)]
#[test]
fn shebang_special_chars() {
    let p = fixtures().join("()%!^&;, ");
    let (out, _) = run_out(p.to_str().unwrap(), &[]).expect("spawn");
    assert_eq!(norm(out.trim()), "special");
}

#[test]
fn shebang_usr_bin_env() {
    let (out, _) = run_out(fixtures().join("shebang").to_str().unwrap(), &[]).expect("spawn");
    assert_eq!(out, "shebang works!");
}

#[test]
fn shebang_via_path_env() {
    let helper_dir = std::path::Path::new(env!("CARGO_BIN_EXE_test_helper"))
        .parent()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let mut c = Command::new("shebang");
    let fixtures_path = fixtures().to_string_lossy().to_string();
    let cur = std::env::var("PATH").unwrap_or_default();
    c.env_clear();
    for (k, v) in std::env::vars() {
        if k.eq_ignore_ascii_case("Path") {
            continue;
        }
        c.env(&k, &v);
    }
    c.env("PATH", format!("{};{};{}", fixtures_path, helper_dir, cur));
    let o = c.output().expect("spawn");
    assert_eq!(String::from_utf8_lossy(&o.stdout), "shebang works!");
}

#[test]
fn empty_and_spaced_args() {
    let helper = env!("CARGO_BIN_EXE_test_helper");
    let (out, _) = run_out(
        helper,
        &["echo", "foo", "", "bar", "André Cruz"],
    )
    .expect("spawn");
    assert_eq!(norm(&out), "foo\n\nbar\nAndré Cruz");
}

#[test]
fn special_char_args() {
    let helper = env!("CARGO_BIN_EXE_test_helper");
    let args = [
        "foo", "()", "foo", "[]", "foo", "%!", "foo", "^<", "foo", ">&", "foo", "|;", "foo",
        ", ", "foo", "!=", "foo", "\\*", "foo", "\"f\"", "foo", "?.", "foo", "=`", "foo", "'",
        "foo", "\\\"", "bar\\",
        "\"(foo|bar>baz)\"", "\"(foo|bar>baz|foz)\"",
    ];
    let mut c = Command::new(helper);
    c.arg("echo");
    c.args(args.iter());
    let o = c.output().unwrap();
    let got = norm(&String::from_utf8_lossy(&o.stdout));
    assert_eq!(got, args.join("\n"));
}

#[cfg(windows)]
#[test]
fn cmd_shim_double_escape() {
    let shim = fixtures().join("node_modules/.bin/echo-cmd-shim");
    let arg = "\"(foo|bar>baz|foz)\"";
    let mut c = Command::new(shim.to_str().unwrap());
    c.double_escape_validator(|path| {
        let lower = path.to_string_lossy().to_lowercase();
        lower.ends_with(".cmd") && lower.contains("node_modules") && lower.contains(".bin")
    });
    c.arg(arg);
    let o = c.output().expect("spawn");
    assert_eq!(norm(&String::from_utf8_lossy(&o.stdout)), arg);
}

#[cfg(windows)]
#[test]
fn env_var_named_command() {
    let p = fixtures().join("%CD%");
    let (out, _) = run_out(p.to_str().unwrap(), &[]).expect("spawn");
    assert_eq!(norm(out.trim()), "special");
}

#[test]
fn exit_code_25() {
    let helper = env!("CARGO_BIN_EXE_test_helper");
    let mut c = Command::new(helper);
    c.args(["exit", "25"]);
    let o = c.output().expect("spawn");
    assert_eq!(o.status.code(), Some(25));
}

#[test]
fn relative_posix_path() {
    let (out, _) = run_out("target/test_fixtures/say-foo", &[]).expect("spawn");
    assert_eq!(norm(out.trim()), "foo");

    let (out, _) = run_out("./target/test_fixtures/say-foo", &[]).expect("spawn");
    assert_eq!(norm(out.trim()), "foo");

    #[cfg(windows)]
    {
        let (out, _) = run_out("./target/test_fixtures/say-foo.bat", &[]).expect("spawn");
        assert_eq!(norm(out.trim()), "foo");
    }
}

#[test]
fn relative_posix_path_custom_cwd() {
    // `test_fixtures/say-foo` is resolved relative to the custom `cwd` ("target").
    let mut c = Command::new("test_fixtures/say-foo");
    c.current_dir("target");
    let o = c.output().expect("spawn");
    assert_eq!(norm(String::from_utf8_lossy(&o.stdout).trim()), "foo");

    let mut c = Command::new("./test_fixtures/say-foo");
    c.current_dir("target");
    let o = c.output().expect("spawn");
    assert_eq!(norm(String::from_utf8_lossy(&o.stdout).trim()), "foo");

    #[cfg(windows)]
    {
        let mut c = Command::new("./test_fixtures/say-foo.bat");
        c.current_dir("target");
        let o = c.output().expect("spawn");
        assert_eq!(norm(String::from_utf8_lossy(&o.stdout).trim()), "foo");
    }
}

#[test]
fn enoent_unknown_command() {
    let mut c = Command::new("somecommandthatwillneverexist");
    c.arg("foo");
    let res = c.spawn();
    assert!(res.is_err());
    match res.unwrap_err() {
        Error::NotFound(_) => {}
        other => panic!("expected NotFound, got: {}", other),
    }
}

#[test]
fn no_enoent_when_command_exists_but_exits_1() {
    let mut c = Command::new(fixtures().join("exit-1").to_str().unwrap());
    let o = c.output().expect("spawn should succeed (no ENOENT)");
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn no_enoent_when_shebang_interpreter_missing() {
    let mut c = Command::new(fixtures().join("shebang-enoent").to_str().unwrap());
    // The file exists; its interpreter is missing. This must NOT be reported as
    // a command-not-found error.
    let o = c.output().expect("spawn should succeed (no ENOENT)");
    assert_ne!(o.status.code(), Some(0));
}

#[cfg(windows)]
#[test]
fn shell_option_expands_percent_random() {
    let mut c = Command::new("echo");
    c.args(["%RANDOM%"]);
    c.shell(true);
    let o = c.output().expect("spawn");
    let out = norm(&String::from_utf8_lossy(&o.stdout)).trim().to_string();
    assert!(out.chars().all(|c| c.is_ascii_digit()), "expected digits, got {:?}", out);
}

#[cfg(not(windows))]
#[test]
fn shell_option_runs_compound_command() {
    let mut c = Command::new("echo");
    c.args(["hello &&", "echo there"]);
    c.shell(true);
    let o = c.output().expect("spawn");
    assert_eq!(norm(&String::from_utf8_lossy(&o.stdout).trim()), "hello\nthere");
}

#[cfg(windows)]
#[test]
fn not_a_shell_for_exe() {
    let helper = env!("CARGO_BIN_EXE_test_helper");
    let mut c = Command::new(helper);
    c.arg("ppid");
    let o = c.output().expect("spawn");
    let ppid = String::from_utf8_lossy(&o.stdout)
        .trim()
        .parse::<u32>()
        .expect("ppid should be a number");
    assert_eq!(ppid, std::process::id());
}

#[cfg(windows)]
#[test]
fn different_path_key_in_env() {
    let fixtures_path = fixtures().to_string_lossy().to_string();
    let cur = std::env::var("PATH").unwrap_or_default();
    let mut c = Command::new("whoami");
    c.env_clear();
    for (k, v) in std::env::vars() {
        if k.eq_ignore_ascii_case("Path") {
            continue;
        }
        c.env(&k, &v);
    }
    // Use the upper-case "PATH" key (different from the canonical "Path").
    c.env("PATH", format!("{};{}", fixtures_path, cur));
    let o = c.output().expect("spawn");
    assert_eq!(norm(&String::from_utf8_lossy(&o.stdout).trim()), "you sure are someone");
}

#[cfg(windows)]
#[test]
fn security_control_characters_rejected_on_windows() {
    // Exists on disk
    let mut c = Command::new(fixtures().join("say-foo").to_str().unwrap());
    c.arg("hello\nworld");
    let res = c.spawn();
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Arguments cannot contain carriage returns or newlines"));
    // Does not exist on disk
    let mut c = Command::new("nonexistent\ncommand");
    let res = c.spawn();
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Command path cannot contain carriage returns or newlines"));

    let mut c = Command::new("echo");
    c.shell(true);
    c.arg("hello\nworld");
    let res = c.spawn();
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Arguments cannot contain carriage returns or newlines"));

    // Env vars
    let mut c = Command::new("echo");
    c.shell(true);
    c.env("KEY\n", "VAL");
    let res = c.spawn();
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Environment variable keys cannot contain carriage returns or newlines"));
}

#[test]
fn security_null_bytes_rejected() {
    // Unconditional NUL byte check
    let mut c = Command::new("echo\0");
    let res = c.spawn();
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Command path cannot contain null bytes"));

    let mut c = Command::new("echo");
    c.arg("hello\0world");
    let res = c.spawn();
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Arguments cannot contain null bytes"));
}
