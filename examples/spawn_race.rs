// Stress test for the spawn/stop race in process_control::spawn_paused.
//
// Spawns many children with spawn_paused, waits until each is actually in
// the stopped state, then checks /proc/<pid>/exe. If it already points at
// the workload binary, the child got past execvp -- i.e. it was running the
// real workload (burning CPU no scheduling decision accounted for) before
// the stop took effect. A correct spawn_paused never lets that happen.
//
// Usage (inside the container, ideally with background CPU load):
//   cargo run -q --example spawn_race -- [count]

#[path = "../src/process_control.rs"]
#[allow(dead_code)]
mod process_control;

use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use std::env;
use std::fs;

fn main() {
    let count: usize = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(500);
    let wpath = env::var("WORKLOAD_PATH").unwrap_or_else(|_| "./target/debug/workload".into());
    let wname = fs::canonicalize(&wpath).expect("workload binary not found");

    let mut ran_early = 0;
    for _ in 0..count {
        let pid = process_control::spawn_paused(&wpath, 1);
        // Block until the child is really stopped (a pending SIGSTOP may not
        // have been acted on yet). WNOHANG would race; WUNTRACED blocks.
        // spawn_paused may have already consumed the stop notification
        // itself, so also accept the child being in state T per /proc.
        loop {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            if stat.rsplit(") ").next().unwrap().starts_with('T') {
                break;
            }
            if let Ok(WaitStatus::Stopped(..)) = waitpid(pid, Some(WaitPidFlag::WUNTRACED | WaitPidFlag::WNOHANG)) {
                break;
            }
            std::thread::yield_now();
        }
        if fs::read_link(format!("/proc/{pid}/exe")).ok().as_deref() == Some(wname.as_path()) {
            ran_early += 1;
        }
        process_control::terminate(pid);
        let _ = waitpid(pid, None);
    }
    println!("{ran_early}/{count} children were already running the workload when stopped");
}
