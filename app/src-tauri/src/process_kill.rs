//! Cross-platform process termination helpers shared by lifecycle recovery code.

/// Send the graceful-shutdown signal to `pid`. Returns `Ok` if the process
/// exited cleanly, was already gone, or accepted the signal. Callers must
/// re-check ownership of the resource (e.g. that the same pid is still bound
/// to the port) before escalating to [`kill_pid_force`].
#[cfg(unix)]
pub(crate) fn kill_pid_term(pid: u32) -> Result<(), String> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let target = Pid::from_raw(pid as i32);
    if let Err(e) = kill(target, Signal::SIGTERM) {
        // ESRCH means already gone — treat as success.
        if e != nix::errno::Errno::ESRCH {
            return Err(format!("SIGTERM pid {pid}: {e}"));
        }
    }
    Ok(())
}

/// Force-kill `pid` after [`kill_pid_term`] failed to free the resource.
/// Caller is responsible for revalidating that `pid` still owns the resource
/// being freed.
#[cfg(unix)]
pub(crate) fn kill_pid_force(pid: u32) -> Result<(), String> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let target = Pid::from_raw(pid as i32);
    match kill(Pid::from_raw(pid as i32), Signal::SIGKILL) {
        Ok(()) => Ok(()),
        // ESRCH means the process exited between our re-validation and the
        // SIGKILL — the resource is freeing on its own, treat as success.
        Err(nix::errno::Errno::ESRCH) => {
            let _ = target;
            Ok(())
        }
        Err(e) => Err(format!("SIGKILL pid {pid}: {e}")),
    }
}

/// Windows has no graceful equivalent for a windowless RPC server — `taskkill`
/// without `/F` only delivers `WM_CLOSE` to GUI apps. Send the WM_CLOSE first
/// (best-effort) so console subprocesses can run shutdown handlers; the
/// follow-up [`kill_pid_force`] does the actual termination.
#[cfg(windows)]
pub(crate) fn kill_pid_term(pid: u32) -> Result<(), String> {
    // Best-effort — ignore non-zero exit (e.g. process is windowless).
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .status();
    Ok(())
}

#[cfg(windows)]
pub(crate) fn kill_pid_force(pid: u32) -> Result<(), String> {
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .status()
        .map_err(|e| format!("taskkill spawn: {e}"))?;
    if !status.success() {
        return Err(format!("taskkill exited with {status}"));
    }
    Ok(())
}
