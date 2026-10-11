//! Wait for signalled Linux process-group members to stop executing.
//!
//! The caller keeps the group leader unreaped during this check, reserving
//! its PID and preventing a new process group from reusing the identifier.

/// Wait until the reserved group contains no executing Linux processes.
#[cfg(target_os = "linux")]
pub(super) async fn wait(pid: u32) -> std::io::Result<()> {
    loop {
        let active = tokio::task::spawn_blocking(move || active_members(pid))
            .await
            .map_err(std::io::Error::other)??;
        if !active {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
}

/// Other platforms retain their existing direct-child acknowledgement.
#[cfg(not(target_os = "linux"))]
pub(super) async fn wait(_: u32) -> std::io::Result<()> {
    Ok(())
}

/// Inspect kernel process states without reaping the reserved group leader.
#[cfg(target_os = "linux")]
fn active_members(group: u32) -> std::io::Result<bool> {
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        // Process entries can disappear or become unreadable between
        // enumeration and reading.
        if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) {
            if is_active_member(&stat, group) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Zombies and dead tasks cannot execute further side effects.
#[cfg(target_os = "linux")]
fn is_active_member(stat: &str, group: u32) -> bool {
    let Some((_, fields)) = stat.rsplit_once(") ") else {
        return false;
    };
    let mut fields = fields.split_whitespace();
    let state = fields.next();
    let process_group = fields.nth(1).and_then(|value| value.parse::<u32>().ok());
    process_group == Some(group) && !matches!(state, Some("Z" | "X" | "x"))
}

#[cfg(all(test, target_os = "linux"))]
#[path = "group_exit_tests.rs"]
mod tests;
