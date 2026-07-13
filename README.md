# cross-spawn

A cross platform solution for spawning processes. Drop-in replacement for `std::process::Command` with Windows-specific enhancements.

[![crates.io](https://img.shields.io/crates/v/cross-spawn.svg)](https://crates.io/crates/cross-spawn)
[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE-MIT)

## Why

Rust's `std::process::Command` works well on Unix but has limitations on Windows:

- **PATHEXT**: Only appends `.exe`, not `.cmd`, `.bat`, `.com`, etc. So `Command::new("code").spawn()` fails even when `code.cmd` exists in PATH.
- **CMD built-ins**: Commands like `echo`, `dir`, `copy` are built into `cmd.exe` and can't be spawned directly via `CreateProcessW`.
- **CREATE_NO_WINDOW**: Requires platform-specific imports (`std::os::windows::process::CommandExt`).
- **Process kill**: No cross-platform kill-by-PID function.

`cross-spawn` fixes all of these transparently.

## Usage

```rust
use cross_spawn::Command;

// Buffered — spawn, collect output, wait
let output = Command::new("npm")
    .args(["install"])
    .env("NODE_ENV", "production")
    .output()?;

// Streaming — returns std::process::Child
let mut child = Command::new("npm")
    .args(["run", "dev"])
    .spawn()?;
let status = child.wait()?;

// Status only
let status = Command::new("make").status()?;

// Windows: hide console window
Command::new("my-app")
    .windows_hide(true)
    .spawn()?;

// Kill by PID
cross_spawn::kill(pid)?;
```

## Platform behavior

| Feature | Windows | Linux / macOS |
|---|---|---|
| PATHEXT resolution | ✅ Auto-resolves `.exe`, `.cmd`, `.bat`, etc. | ❌ No-op (not applicable) |
| CMD built-in detection | ✅ Auto-wraps in `cmd /c` | ❌ No-op |
| `windows_hide()` | ✅ Sets `CREATE_NO_WINDOW` | ❌ No-op (method not available) |
| `kill(pid)` | `taskkill /T /F` (tree kill) | `kill -TERM` |
| Everything else | Pass-through to `std::process::Command` | Pass-through |

## API

`cross_spawn::Command` mirrors `std::process::Command` exactly:

- `new()`, `arg()`, `args()`
- `env()`, `env_remove()`, `env_clear()`
- `current_dir()`
- `stdin()`, `stdout()`, `stderr()`
- `spawn()`, `output()`, `status()`
- Windows-only: `windows_hide()`

Plus the free function:

- `cross_spawn::kill(pid)` — cross-platform process termination

## Development

### Running Tests
The test suite is written in pure Rust and does not require Node.js or any external runtimes. Test fixtures are generated dynamically during the test execution in `target/test_fixtures/` and `target/unit_fixtures/`.

To run the unit and integration tests:
```bash
cargo test
```

### Running Examples
You can run the demonstration example to verify process spawning:
```bash
cargo run --example demo
```

## License

MIT
