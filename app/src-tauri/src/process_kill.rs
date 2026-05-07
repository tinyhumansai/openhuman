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

/// Send SIGTERM, then SIGKILL holdouts, to every direct child of the
/// current process. No-op on non-Unix platforms (Windows job objects already
/// kill CEF helpers when the parent exits).
pub(crate) fn sweep_orphan_children() {
    #[cfg(unix)]
    {
        sweep_orphan_children_unix(std::process::id());
    }
    #[cfg(not(unix))]
    {
        log::debug!("[app] sweep: skipped on non-unix platform");
    }
}

#[cfg(unix)]
fn sweep_orphan_children_unix(parent_pid: u32) {
    let term_count = match direct_child_pids(parent_pid) {
        Ok(pids) => pids.len(),
        Err(err) => {
            log::warn!("[app] sweep: failed to enumerate children before SIGTERM: {err}");
            0
        }
    };

    let term_signaled = match pkill_children(parent_pid, "TERM") {
        Ok(status) => {
            let signaled = signaled_at_least_one(&status);
            log_unexpected_pkill_status("SIGTERM", status);
            signaled
        }
        Err(err) => {
            log::warn!("[app] sweep: failed to invoke pkill SIGTERM: {err}");
            false
        }
    };
    if term_count > 0 || term_signaled {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let kill_count = match direct_child_pids(parent_pid) {
        Ok(pids) => pids.len(),
        Err(err) => {
            log::warn!("[app] sweep: failed to enumerate children after SIGTERM: {err}");
            0
        }
    };

    match pkill_children(parent_pid, "KILL") {
        Ok(status) => log_unexpected_pkill_status("SIGKILL", status),
        Err(err) => log::warn!("[app] sweep: failed to invoke pkill SIGKILL: {err}"),
    }

    let total = term_count + kill_count;
    if kill_count > 0 {
        log::warn!("[app] sweep: term={term_count} kill={kill_count} total={total}");
    } else {
        log::info!("[app] sweep: term={term_count} kill=0 total={total}");
    }
}

#[cfg(unix)]
fn direct_child_pids(parent_pid: u32) -> Result<Vec<u32>, String> {
    let output = std::process::Command::new("pgrep")
        .args(["-P", &parent_pid.to_string()])
        .output()
        .map_err(|err| format!("spawn pgrep: {err}"))?;

    match output.status.code() {
        Some(0) => Ok(parse_pgrep_pids(&String::from_utf8_lossy(&output.stdout))),
        Some(1) => Ok(Vec::new()),
        other => Err(format!("pgrep exited with {other:?}")),
    }
}

#[cfg(unix)]
fn parse_pgrep_pids(stdout: &str) -> Vec<u32> {
    stdout
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

#[cfg(unix)]
fn pkill_children(parent_pid: u32, signal: &str) -> Result<std::process::ExitStatus, String> {
    let signal_arg = format!("-{signal}");
    let parent_pid = parent_pid.to_string();
    std::process::Command::new("pkill")
        .args([signal_arg.as_str(), "-P", parent_pid.as_str()])
        .status()
        .map_err(|err| format!("spawn pkill -{signal}: {err}"))
}

#[cfg(unix)]
fn log_unexpected_pkill_status(signal_name: &str, status: std::process::ExitStatus) {
    // pkill exits 0 if it signaled at least one process, 1 if no process
    // matched. Both are valid because children can exit between pgrep and
    // pkill; other statuses are real command failures.
    match status.code() {
        Some(0) | Some(1) => {}
        other => log::warn!("[app] sweep: pkill {signal_name} exited with {other:?}"),
    }
}

#[cfg(unix)]
fn signaled_at_least_one(status: &std::process::ExitStatus) -> bool {
    matches!(status.code(), Some(0))
}

/// Windows has no graceful equivalent for a windowless RPC server — `taskkill`
/// without `/F` only delivers `WM_CLOSE` to GUI apps. Send the WM_CLOSE first
/// (best-effort) so console subprocesses can run shutdown handlers; the
/// follow-up [`kill_pid_force`] does the actual termination.
#[cfg(windows)]
pub(crate) fn kill_pid_term(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Best-effort — ignore non-zero exit (e.g. process is windowless).
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    Ok(())
}

#[cfg(windows)]
pub(crate) fn kill_pid_force(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("taskkill spawn: {e}"))?;
    if !status.success() {
        return Err(format!("taskkill exited with {status}"));
    }
    Ok(())
}
