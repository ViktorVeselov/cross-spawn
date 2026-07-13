use cross_spawn::Command;

fn main() {
    println!("--- Spawning 'echo' with cross-spawn ---");
    
    let mut cmd = Command::new("echo");
    cmd.args(["Hello", "from", "our", "cross-spawn", "library", "written", "in", "Rust!"]);
    
    let mut child = cmd.spawn().expect("Failed to spawn process");
    let status = child.wait().expect("Failed to wait on child");
    
    println!("Exit status: {:?}", status);
}
