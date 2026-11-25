//! Cross-platform process termination helpers shared by lifecycle recovery code.

/// The raw pid to signal, refusing values `kill(2)` would not treat as one
/// process: 0 (the caller's whole process group), 1 (init/launchd), and
/// anything above `i32::MAX`, which `as i32` would turn negative — a process
/// group again.
#[cfg(unix)]
fn signal_pid(pid: u32) -> Result<i32, String> {
    match i32::try_from(pid) {
        Ok(raw) if raw > 1 => Ok(raw),
        _ => Err(format!("refusing to signal protected or invalid pid {pid}")),
    }
}

/// Send the graceful-shutdown signal to `pid`. Returns `Ok` if the process
/// exited cleanly, was already gone, or accepted the signal. Callers must
/// re-check ownership of the resource (e.g. that the same pid is still bound
/// to the port) before escalating to [`kill_pid_force`].
#[cfg(unix)]
pub(crate) fn kill_pid_term(pid: u32) -> Result<(), String> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let target = Pid::from_raw(signal_pid(pid)?);
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
    let target = Pid::from_raw(signal_pid(pid)?);
    match kill(target, Signal::SIGKILL) {
        Ok(()) => Ok(()),
        // ESRCH means the process exited between our re-validation and the
        // SIGKILL — the resource is freeing on its own, treat as success.
        Err(nix::errno::Errno::ESRCH) => Ok(()),
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
///
/// Refuses to signal the protected system PIDs 0 (System Idle Process) and 4
/// (NT Kernel & System) — those should never be reachable from
/// `find_pid_on_port`, but if they slip through the parser they would
/// otherwise produce a hard taskkill failure that aborts startup recovery.
#[cfg(windows)]
pub(crate) fn kill_pid_term(pid: u32) -> Result<(), String> {
    if is_protected_windows_pid(pid) {
        return Err(format!("refusing to signal protected windows pid {pid}"));
    }
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
    if is_protected_windows_pid(pid) {
        return Err(format!(
            "refusing to force-kill protected windows pid {pid}"
        ));
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("taskkill spawn: {e}"))?;
    classify_taskkill_force_status(output.status.code(), &output.stderr, pid)
}

/// Force-kill `pid` WITHOUT the `/T` tree flag, so only the target process is
/// terminated and its child tree is left untouched.
///
/// The pre-CEF stale reap (issue #3900) targets a wedged GUI browser process
/// specifically. `/T` is both unnecessary and dangerous there: the OS job
/// object already reaps that process's CEF helper children when it exits, and a
/// tree walk could reach the freshly launched app if the stale process is an
/// ancestor of ours during an auto-update relaunch. Callers must revalidate the
/// pid still owns the resource before calling. Shares the "already gone"
/// success classification with [`kill_pid_force`].
#[cfg(windows)]
pub(crate) fn kill_pid_force_no_tree(pid: u32) -> Result<(), String> {
    if is_protected_windows_pid(pid) {
        return Err(format!(
            "refusing to force-kill protected windows pid {pid}"
        ));
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("taskkill spawn: {e}"))?;
    classify_taskkill_force_status(output.status.code(), &output.stderr, pid)
}

/// Classify a `taskkill /F /T /PID <pid>` exit. Exit code 128 ("process not
/// found") means the process already exited between the pid lookup and the
/// force-kill — the resource is freeing on its own, treat as success. Same
/// semantics as ESRCH on Unix (`kill_pid_force` returns Ok for that case).
///
/// `stderr` is matched as a fallback when exit codes are masked by an
/// intermediate shell — some Windows hosts/wrappers normalize taskkill exit
/// codes to 1 but still write the "not found" message to stderr.
#[cfg(windows)]
pub(crate) fn classify_taskkill_force_status(
    code: Option<i32>,
    stderr: &[u8],
    pid: u32,
) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        // 128 = "There is no running instance of the task." — process already gone.
        Some(128) => {
            log::debug!("[app] taskkill /F /PID {pid}: process already gone (exit 128)");
            Ok(())
        }
        other => {
            let stderr_str = String::from_utf8_lossy(stderr);
            // Only treat the "process is gone" stderr shapes as success.
            // `could not be terminated` ALONE is *not* enough — it also
            // appears in access-denied messages like
            // "could not be terminated. Reason: Access is denied." which
            // we must surface as a real failure.
            let stderr_lower = stderr_str.to_ascii_lowercase();
            let process_gone = stderr_lower.contains("no running instance of the task")
                || (stderr_lower.contains("could not be terminated")
                    && stderr_lower.contains("not found"))
                || (stderr_lower.contains("error: the process")
                    && stderr_lower.contains("not found"));
            if process_gone {
                log::debug!(
                    "[app] taskkill /F /PID {pid}: process already gone (stderr match: {stderr_str:?})"
                );
                return Ok(());
            }
            Err(format!(
                "taskkill exited with code {other:?} stderr={stderr_str:?}"
            ))
        }
    }
}

/// PIDs 0 (System Idle Process) and 4 (NT Kernel & System) are kernel-owned
/// and cannot be signalled by user-mode processes. They occasionally surface
/// in `netstat -ano` output for sockets reserved by HTTP.sys or other
/// kernel-side bindings — guard against ever trying to kill them.
#[cfg(windows)]
pub(crate) const fn is_protected_windows_pid(pid: u32) -> bool {
    pid == 0 || pid == 4
}

#[cfg(all(test, unix))]
#[path = "process_kill_unix_tests.rs"]
mod unix_tests;

#[cfg(test)]
#[path = "process_kill_tests.rs"]
mod tests;
