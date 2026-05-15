//! Spawn marker for openhuman-owned `ollama serve` processes.
//!
//! Every time `start_and_wait_for_server` actually spawns an Ollama daemon
//! (i.e. didn't adopt a healthy external one), we write a small JSON file
//! recording the PID, the binary we launched, and the openhuman process
//! that owned it. On graceful shutdown the marker is cleared. If openhuman
//! crashes before its shutdown hook fires, the marker survives — and on
//! next launch we can reclaim the orphaned daemon (kill + respawn fresh)
//! instead of either leaking it forever or running blanket `taskkill /IM
//! ollama.exe` and hitting daemons we don't own.
//!
//! Liveness is checked via `sysinfo` (already a workspace dep) to match
//! the cross-platform pattern in `install::is_ollama_installer_running`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::paths::ollama_spawn_marker_path;

/// On-disk record of an openhuman-spawned `ollama serve` process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OllamaSpawnMarker {
    pub pid: u32,
    pub started_at_unix: u64,
    pub binary_path: String,
    pub openhuman_pid: u32,
}

impl OllamaSpawnMarker {
    pub(crate) fn new(pid: u32, binary_path: &Path) -> Self {
        let started_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            pid,
            started_at_unix,
            binary_path: binary_path.display().to_string(),
            openhuman_pid: std::process::id(),
        }
    }
}

// ---- Path-keyed helpers (testable without touching ~/.openhuman/) -------

/// Write `marker` to `path`, replacing any existing file. Creates the
/// parent directory if needed. Uses a tmp-and-rename so a crash mid-write
/// can't leave truncated JSON.
pub(crate) fn write_marker_at(path: &Path, marker: &OllamaSpawnMarker) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create marker dir {}: {e}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(marker).map_err(|e| format!("serialize spawn marker: {e}"))?;

    let tmp = path.with_extension("spawn.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write marker tmp {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename marker {} -> {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

pub(crate) fn read_marker_at(path: &Path) -> Option<OllamaSpawnMarker> {
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<OllamaSpawnMarker>(&content) {
        Ok(m) => Some(m),
        Err(e) => {
            log::warn!(
                "[local_ai] ollama spawn marker at {} is unparseable, ignoring: {e}",
                path.display()
            );
            None
        }
    }
}

pub(crate) fn clear_marker_at(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => log::debug!(
            "[local_ai] cleared ollama spawn marker at {}",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!(
            "[local_ai] failed to clear ollama spawn marker {}: {e}",
            path.display()
        ),
    }
}

// ---- Config-keyed conveniences (used by production code) ---------------

pub(crate) fn write_marker(config: &Config, marker: &OllamaSpawnMarker) -> Result<(), String> {
    let path = ollama_spawn_marker_path(config);
    write_marker_at(&path, marker)?;
    log::debug!(
        "[local_ai] wrote ollama spawn marker pid={} bin={} at {}",
        marker.pid,
        marker.binary_path,
        path.display()
    );
    Ok(())
}

pub(crate) fn read_marker(config: &Config) -> Option<OllamaSpawnMarker> {
    read_marker_at(&ollama_spawn_marker_path(config))
}

pub(crate) fn clear_marker(config: &Config) {
    clear_marker_at(&ollama_spawn_marker_path(config));
}

/// True iff `pid` corresponds to a live process on this machine.
///
/// Uses `sysinfo` rather than libc/Win32 directly to stay consistent with
/// the rest of the local_ai module (see `install::is_ollama_installer_running`).
pub(crate) fn pid_is_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut sys = System::new();
    let target = Pid::from_u32(pid);
    // Refresh just the one PID we care about; cheap on Windows where a full
    // refresh can take ~tens of ms on a loaded machine.
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    sys.process(target).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_marker_path() -> (TempDir, std::path::PathBuf) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("local-ai").join("ollama.spawn");
        (tmp, path)
    }

    #[test]
    fn marker_round_trips_through_disk() {
        let (_tmp, path) = tmp_marker_path();
        let m = OllamaSpawnMarker {
            pid: 4242,
            started_at_unix: 1_700_000_000,
            binary_path: "C:\\fake\\ollama.exe".to_string(),
            openhuman_pid: 9001,
        };

        write_marker_at(&path, &m).expect("write marker");
        let loaded = read_marker_at(&path).expect("read marker");
        assert_eq!(loaded, m);

        clear_marker_at(&path);
        assert!(
            read_marker_at(&path).is_none(),
            "marker must be gone after clear"
        );
    }

    #[test]
    fn read_marker_returns_none_when_file_missing() {
        let (_tmp, path) = tmp_marker_path();
        assert!(read_marker_at(&path).is_none());
    }

    #[test]
    fn read_marker_returns_none_on_corrupt_json() {
        let (_tmp, path) = tmp_marker_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not valid json").unwrap();

        assert!(
            read_marker_at(&path).is_none(),
            "corrupt marker must be treated as absent"
        );
    }

    #[test]
    fn clear_marker_is_idempotent() {
        let (_tmp, path) = tmp_marker_path();
        clear_marker_at(&path);
        clear_marker_at(&path);
    }

    #[test]
    fn write_marker_creates_missing_parent_dir() {
        let (_tmp, path) = tmp_marker_path();
        // path.parent() does NOT exist yet — write should create it.
        assert!(!path.parent().unwrap().exists());
        let m = OllamaSpawnMarker::new(1234, std::path::Path::new("ollama"));
        write_marker_at(&path, &m).expect("write");
        assert!(path.exists());
    }

    #[test]
    fn new_marker_captures_current_process_id() {
        let m = OllamaSpawnMarker::new(4242, std::path::Path::new("ollama"));
        assert_eq!(m.openhuman_pid, std::process::id());
        assert_eq!(m.pid, 4242);
        assert_eq!(m.binary_path, "ollama");
    }

    #[test]
    fn pid_is_alive_recognises_self() {
        let me = std::process::id();
        assert!(
            pid_is_alive(me),
            "current process PID {me} should be reported alive"
        );
    }

    #[test]
    fn pid_is_alive_rejects_dead_pid() {
        // Spawn a short child, wait for it to exit, then check that its
        // recycled PID is no longer reported alive. Hardcoded sentinel PIDs
        // (0, u32::MAX) are unreliable cross-platform — on Windows PID 0 is
        // "System Idle Process" and registers as alive in sysinfo.
        let child = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", "exit 0"])
                .spawn()
                .expect("spawn cmd /C exit")
        } else {
            std::process::Command::new("true")
                .spawn()
                .expect("spawn /usr/bin/true")
        };
        let pid = child.id();
        let mut child = child;
        let _ = child.wait();

        // Give the OS a moment to fully reap so sysinfo doesn't catch a
        // lingering zombie entry.
        std::thread::sleep(std::time::Duration::from_millis(200));

        assert!(
            !pid_is_alive(pid),
            "exited child pid {pid} should not be reported alive"
        );
    }
}
