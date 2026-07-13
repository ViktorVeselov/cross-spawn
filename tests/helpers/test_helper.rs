use std::env;
use std::process;

#[cfg(windows)]
fn get_parent_pid() -> Option<u32> {
    use std::mem;
    type HANDLE = *mut std::ffi::c_void;
    const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
    const TH32CS_SNAPPROCESS: u32 = 0x00000002;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESSENTRY32 {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u8; 260],
    }

    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> HANDLE;
        fn CloseHandle(hObject: HANDLE) -> i32;
        fn Process32First(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32) -> i32;
        fn Process32Next(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32) -> i32;
        fn GetCurrentProcessId() -> u32;
    }

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32 = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32>() as u32;
        let my_pid = GetCurrentProcessId();
        let mut ppid = None;
        if Process32First(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == my_pid {
                    ppid = Some(entry.th32ParentProcessID);
                    break;
                }
                if Process32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        ppid
    }
}

#[cfg(not(windows))]
fn get_parent_pid() -> Option<u32> {
    unsafe extern "C" {
        fn getppid() -> i32;
    }
    unsafe { Some(getppid() as u32) }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: test_helper <command> [args...]");
        process::exit(1);
    }
    match args[1].as_str() {
        "echo" => {
            let joined = args[2..].join("\n");
            print!("{}", joined);
        }
        "exit" => {
            let code = args.get(2)
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);
            process::exit(code);
        }
        "ppid" => {
            if let Some(ppid) = get_parent_pid() {
                println!("{}", ppid);
            } else {
                eprintln!("failed to get ppid");
                process::exit(1);
            }
        }
        other => {
            let path = std::path::Path::new(other);
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                match name {
                    "shebang" => {
                        print!("shebang works!");
                        return;
                    }
                    "say-foo" => {
                        print!("foo");
                        return;
                    }
                    "()%!^&;, " => {
                        print!("special");
                        return;
                    }
                    "%CD%" => {
                        print!("special");
                        return;
                    }
                    _ => {}
                }
            }
            eprintln!("Unknown command: {}", other);
            process::exit(1);
        }
    }
}
