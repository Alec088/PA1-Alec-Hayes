// process_control.rs -- provided scaffold.
//
// Thin wrappers around the OS primitives this assignment requires you to
// use directly: fork/exec, SIGSTOP/SIGCONT, waitpid, and a monotonic
// clock. These functions perform the syscalls for you; the scheduling
// *policy* -- when to call them, and in what order -- is the part you're
// responsible for. Using these correctly still requires you to reason
// about races (e.g. a child can exit before you get around to reaping
// it, a stop signal can race with process startup) -- the wrappers don't
// hide that reasoning from you, only the raw libc calls.

use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{execvp, fork, ForkResult, Pid};
use std::ffi::CString;
use std::time::Instant;

/// Fork + exec `workload <ticks>`, immediately stopped (SIGSTOP) so it
/// consumes no CPU until the scheduler explicitly resumes it.
/// Returns the child's PID.
///
/// Race fix: the original version sent SIGSTOP from the parent after fork.
/// Nothing guarantees the parent runs before the child, so under load the
/// child could get through execvp and start the real workload before the
/// stop landed, consuming CPU no scheduling decision accounted for. Waiting
/// for the stop in the parent doesn't help -- it only confirms the child
/// stopped *eventually*, not *before exec*. Instead the child stops itself
/// before exec, so it cannot reach the workload until it is SIGCONTed.
/// The parent then waits (WUNTRACED) for that stop: otherwise a SIGCONT
/// sent before the child reached raise() would be lost and the child would
/// stay stopped forever.
pub fn spawn_paused(workload_path: &str, ticks: u64) -> Pid {
    // Build exec arguments before forking so the child does no allocation.
    let prog = CString::new(workload_path).unwrap();
    let args = [prog.clone(), CString::new(ticks.to_string()).unwrap()];
    match unsafe { fork() }.expect("fork failed") {
        ForkResult::Parent { child } => {
            loop {
                match waitpid(child, Some(WaitPidFlag::WUNTRACED)) {
                    Ok(WaitStatus::Stopped(_, Signal::SIGSTOP)) => break,
                    Err(Errno::EINTR) => continue,
                    other => panic!("child {child} did not stop before exec: {other:?}"),
                }
            }
            child
        }
        ForkResult::Child => {
            unsafe { libc::raise(libc::SIGSTOP) };
            // Only reached after the scheduler resumes us.
            let _ = execvp(&prog, &args);
            // exec failed: exit without running the parent's atexit handlers.
            unsafe { libc::_exit(127) };
        }
    }
}

/// Resume a stopped process -- your scheduler "selecting" it.
pub fn resume(pid: Pid) {
    kill(pid, Signal::SIGCONT).expect("failed to resume child");
}

/// Pause a running process -- your scheduler "preempting" it.
pub fn preempt(pid: Pid) {
    kill(pid, Signal::SIGSTOP).expect("failed to stop child");
}

/// Non-blocking check for whether a child has exited. Returns true if it
/// has (and reaps it in the process). You are responsible for calling
/// this, or an equivalent blocking wait, for every child you spawn -- an
/// exited child you never reap is a zombie.
pub fn try_reap(pid: Pid) -> bool {
    matches!(
        waitpid(pid, Some(WaitPidFlag::WNOHANG)),
        Ok(WaitStatus::Exited(_, _))
    )
}

/// Forcibly terminate a child (SIGKILL) and reap it. Used during cleanup
/// for any process still alive when the run ends -- e.g. one that "did not
/// finish" within runfor. SIGKILL alone only makes the child a zombie; it
/// stays in the process table until we collect its status, so we reap it
/// here. (SIGKILL takes effect even on a SIGSTOPped process.)
pub fn terminate(pid: Pid) {
    let _ = kill(pid, Signal::SIGKILL);
    reap_blocking(pid);
}

/// Blocking reap -- waits for the given child to actually exit. Used
/// during cleanup, and to confirm a selected process's completion before
/// you log it as finished.
///
/// A blocking waitpid can fail with EINTR if a signal arrives while it
/// waits; that means "nothing collected yet", not "child exited", so we
/// retry until we really have its exit (or death-by-signal) status.
pub fn reap_blocking(pid: Pid) {
    loop {
        match waitpid(pid, None) {
            Ok(WaitStatus::Exited(..)) | Ok(WaitStatus::Signaled(..)) => return,
            Err(Errno::EINTR) => continue,
            // Already reaped / not our child: nothing left to collect.
            Err(Errno::ECHILD) => return,
            // Any other status (e.g. stopped/continued) isn't an exit.
            Ok(_) => continue,
            Err(e) => panic!("waitpid({pid}) failed: {e}"),
        }
    }
}

/// Milliseconds elapsed since a fixed reference point you establish at
/// program start. Use this -- not the input file's tick numbers -- to
/// compute every event time you print in your output.
pub fn now_ms(epoch: &Instant) -> u64 {
    epoch.elapsed().as_millis() as u64
}
