//! macOS Globe/Fn key listener helper management.
//!
//! The listener runs as a tiny Swift process that monitors `flagsChanged`
//! events globally and reports `FN_DOWN` / `FN_UP` lines over stdout.

use super::{detect_permissions, PermissionState};
#[cfg(target_os = "macos")]
use std::collections::VecDeque;

#[cfg(target_os = "macos")]
use once_cell::sync::Lazy;
#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::io::{BufRead, BufReader};
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex as StdMutex};

const LOG_PREFIX: &str = "[globe_hotkey]";
const MAX_PENDING_EVENTS: usize = 64;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlobeHotkeyStatus {
    pub supported: bool,
    pub running: bool,
    pub input_monitoring_permission: PermissionState,
    pub last_error: Option<String>,
    pub events_pending: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlobeHotkeyPollResult {
    pub status: GlobeHotkeyStatus,
    pub events: Vec<String>,
}

#[cfg(target_os = "macos")]
struct GlobeListenerProcess {
    child: Child,
    event_queue: Arc<StdMutex<VecDeque<String>>>,
    last_error: Arc<StdMutex<Option<String>>>,
}

#[cfg(target_os = "macos")]
static GLOBE_LISTENER: Lazy<StdMutex<Option<GlobeListenerProcess>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg(target_os = "macos")]
fn push_event(queue: &Arc<StdMutex<VecDeque<String>>>, event: String) {
    let Ok(mut guard) = queue.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock queue for event");
        return;
    };
    guard.push_back(event);
    while guard.len() > MAX_PENDING_EVENTS {
        let _ = guard.pop_front();
    }
}

#[cfg(target_os = "macos")]
fn set_last_error(error_store: &Arc<StdMutex<Option<String>>>, message: Option<String>) {
    let Ok(mut guard) = error_store.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock last_error store");
        return;
    };
    *guard = message;
}

#[cfg(target_os = "macos")]
fn drain_events(queue: &Arc<StdMutex<VecDeque<String>>>) -> Vec<String> {
    let Ok(mut guard) = queue.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock queue for drain");
        return Vec::new();
    };
    guard.drain(..).collect()
}

#[cfg(target_os = "macos")]
fn queue_len(queue: &Arc<StdMutex<VecDeque<String>>>) -> usize {
    let Ok(guard) = queue.lock() else {
        return 0;
    };
    guard.len()
}

#[cfg(target_os = "macos")]
fn current_error(error_store: &Arc<StdMutex<Option<String>>>) -> Option<String> {
    let Ok(guard) = error_store.lock() else {
        return Some("failed to read globe listener error state".to_string());
    };
    guard.clone()
}

#[cfg(target_os = "macos")]
fn ensure_running_locked(
    state: &mut Option<GlobeListenerProcess>,
) -> Result<GlobeHotkeyStatus, String> {
    let input_monitoring_permission = detect_permissions().input_monitoring;
    if input_monitoring_permission != PermissionState::Granted {
        let message =
            "input monitoring permission is required for the macOS Globe/Fn listener".to_string();
        log::warn!(
            "{LOG_PREFIX} start skipped: input_monitoring_permission={:?}",
            input_monitoring_permission
        );
        if let Some(process) = state.as_ref() {
            set_last_error(&process.last_error, Some(message.clone()));
        }
        return Ok(GlobeHotkeyStatus {
            supported: true,
            running: false,
            input_monitoring_permission,
            last_error: Some(message),
            events_pending: 0,
        });
    }

    if let Some(process) = state.as_mut() {
        match process.child.try_wait() {
            Ok(None) => {
                return Ok(GlobeHotkeyStatus {
                    supported: true,
                    running: true,
                    input_monitoring_permission,
                    last_error: current_error(&process.last_error),
                    events_pending: queue_len(&process.event_queue),
                });
            }
            Ok(Some(status)) => {
                let message = format!("globe listener exited unexpectedly: {status}");
                log::warn!("{LOG_PREFIX} {message}");
                set_last_error(&process.last_error, Some(message));
                *state = None;
            }
            Err(err) => {
                let message = format!("failed to inspect globe listener state: {err}");
                log::warn!("{LOG_PREFIX} {message}");
                set_last_error(&process.last_error, Some(message));
                *state = None;
            }
        }
    }

    let binary_path = ensure_globe_helper_binary()?;
    log::info!("{LOG_PREFIX} starting helper {}", binary_path.display());
    let mut child = Command::new(&binary_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn globe listener helper: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture globe listener stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture globe listener stderr".to_string())?;

    let event_queue = Arc::new(StdMutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)));
    let last_error = Arc::new(StdMutex::new(None));

    {
        let queue = event_queue.clone();
        let error_store = last_error.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        log::debug!("{LOG_PREFIX} helper event={trimmed}");
                        push_event(&queue, trimmed.to_string());
                        set_last_error(&error_store, None);
                    }
                    Err(err) => {
                        let message = format!("failed reading globe listener stdout: {err}");
                        log::warn!("{LOG_PREFIX} {message}");
                        set_last_error(&error_store, Some(message));
                        break;
                    }
                }
            }
            log::debug!("{LOG_PREFIX} stdout reader exited");
        });
    }

    {
        let error_store = last_error.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        log::warn!("{LOG_PREFIX} helper stderr={trimmed}");
                        set_last_error(&error_store, Some(trimmed.to_string()));
                    }
                    Err(err) => {
                        let message = format!("failed reading globe listener stderr: {err}");
                        log::warn!("{LOG_PREFIX} {message}");
                        set_last_error(&error_store, Some(message));
                        break;
                    }
                }
            }
            log::debug!("{LOG_PREFIX} stderr reader exited");
        });
    }

    *state = Some(GlobeListenerProcess {
        child,
        event_queue,
        last_error,
    });

    let process = state
        .as_ref()
        .ok_or_else(|| "globe listener process missing after spawn".to_string())?;
    Ok(GlobeHotkeyStatus {
        supported: true,
        running: true,
        input_monitoring_permission,
        last_error: current_error(&process.last_error),
        events_pending: queue_len(&process.event_queue),
    })
}

#[cfg(target_os = "macos")]
fn ensure_globe_helper_binary() -> Result<PathBuf, String> {
    let cache_dir = std::env::temp_dir().join("openhuman-globe-listener");
    fs::create_dir_all(&cache_dir).map_err(|e| format!("failed to create globe cache dir: {e}"))?;

    let source_path = cache_dir.join("globe_listener.swift");
    let binary_path = cache_dir.join("globe_listener_bin");
    let source = globe_swift_source();

    let needs_write = match fs::read_to_string(&source_path) {
        Ok(existing) => existing != source,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&source_path, &source)
            .map_err(|e| format!("failed to write globe helper source: {e}"))?;
    }

    let needs_compile = needs_write || !binary_path.exists();
    if needs_compile {
        log::debug!("{LOG_PREFIX} compiling Swift helper");
        let output = Command::new("xcrun")
            .args(["swiftc", "-O", "-framework", "Cocoa"])
            .arg(&source_path)
            .arg("-o")
            .arg(&binary_path)
            .output()
            .or_else(|_| {
                Command::new("swiftc")
                    .args(["-O", "-framework", "Cocoa"])
                    .arg(&source_path)
                    .arg("-o")
                    .arg(&binary_path)
                    .output()
            })
            .map_err(|e| format!("failed to invoke swiftc for globe listener: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "failed to compile globe listener helper: {}",
                if stderr.is_empty() {
                    "swiftc returned non-zero exit status".to_string()
                } else {
                    stderr
                }
            ));
        }
        log::debug!("{LOG_PREFIX} Swift helper compiled successfully");
    }

    Ok(binary_path)
}

#[cfg(target_os = "macos")]
fn globe_swift_source() -> String {
    r##"import Cocoa
import Darwin

var fnIsDown = false
var lastModifierFlags: NSEvent.ModifierFlags = []

let rightModifiers: [(UInt16, NSEvent.ModifierFlags, String)] = [
    (61, .option, "RightOption"),
    (54, .command, "RightCommand"),
    (62, .control, "RightControl"),
    (60, .shift, "RightShift"),
]

let modifierMask: NSEvent.ModifierFlags = [.control, .command, .option, .shift]

let releases: [(NSEvent.ModifierFlags, String)] = [
    (.control, "control"),
    (.command, "command"),
    (.option, "option"),
    (.shift, "shift"),
]

func emit(_ message: String) {
    FileHandle.standardOutput.write((message + "\n").data(using: .utf8)!)
    fflush(stdout)
}

guard let monitor = NSEvent.addGlobalMonitorForEvents(matching: .flagsChanged, handler: { event in
    let flags = event.modifierFlags
    let containsFn = flags.contains(.function)

    if containsFn && !fnIsDown {
        fnIsDown = true
        emit("FN_DOWN")
    } else if !containsFn && fnIsDown {
        fnIsDown = false
        emit("FN_UP")
    }

    let keyCode = event.keyCode
    for (code, flag, name) in rightModifiers {
        if keyCode == code {
            emit(flags.contains(flag) ? "RIGHT_MOD_DOWN:\(name)" : "RIGHT_MOD_UP:\(name)")
            break
        }
    }

    let currentModifiers = flags.intersection(modifierMask)
    if currentModifiers != lastModifierFlags {
        let released = lastModifierFlags.subtracting(currentModifiers)
        for (flag, name) in releases {
            if released.contains(flag) {
                emit("MODIFIER_UP:\(name)")
            }
        }
        lastModifierFlags = currentModifiers
    }
}) else {
    FileHandle.standardError.write("Failed to create event monitor\n".data(using: .utf8)!)
    exit(1)
}

let signalSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
signal(SIGTERM, SIG_IGN)
signalSource.setEventHandler {
    NSEvent.removeMonitor(monitor)
    exit(0)
}
signalSource.resume()

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
app.run()
"##
    .to_string()
}

#[cfg(target_os = "macos")]
pub fn globe_listener_start() -> Result<GlobeHotkeyStatus, String> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| "globe listener lock poisoned".to_string())?;
    ensure_running_locked(&mut guard)
}

#[cfg(target_os = "macos")]
pub fn globe_listener_poll() -> Result<GlobeHotkeyPollResult, String> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| "globe listener lock poisoned".to_string())?;
    let status = ensure_running_locked(&mut guard)?;
    let events = guard
        .as_ref()
        .map(|process| drain_events(&process.event_queue))
        .unwrap_or_default();
    Ok(GlobeHotkeyPollResult {
        status: GlobeHotkeyStatus {
            events_pending: 0,
            ..status
        },
        events,
    })
}

#[cfg(target_os = "macos")]
pub fn globe_listener_stop() -> Result<GlobeHotkeyStatus, String> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| "globe listener lock poisoned".to_string())?;
    if let Some(mut process) = guard.take() {
        log::info!("{LOG_PREFIX} stopping helper pid={}", process.child.id());
        let _ = process.child.kill();
        let _ = process.child.wait();
        let events = drain_events(&process.event_queue);
        log::debug!(
            "{LOG_PREFIX} drained {} queued events on stop",
            events.len()
        );
    }

    Ok(GlobeHotkeyStatus {
        supported: true,
        running: false,
        input_monitoring_permission: detect_permissions().input_monitoring,
        last_error: None,
        events_pending: 0,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn globe_listener_start() -> Result<GlobeHotkeyStatus, String> {
    Ok(GlobeHotkeyStatus {
        supported: false,
        running: false,
        input_monitoring_permission: detect_permissions().input_monitoring,
        last_error: Some("Globe/Fn hotkey listener is only supported on macOS".to_string()),
        events_pending: 0,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn globe_listener_poll() -> Result<GlobeHotkeyPollResult, String> {
    Ok(GlobeHotkeyPollResult {
        status: GlobeHotkeyStatus {
            supported: false,
            running: false,
            input_monitoring_permission: detect_permissions().input_monitoring,
            last_error: Some("Globe/Fn hotkey listener is only supported on macOS".to_string()),
            events_pending: 0,
        },
        events: Vec::new(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn globe_listener_stop() -> Result<GlobeHotkeyStatus, String> {
    Ok(GlobeHotkeyStatus {
        supported: false,
        running: false,
        input_monitoring_permission: detect_permissions().input_monitoring,
        last_error: Some("Globe/Fn hotkey listener is only supported on macOS".to_string()),
        events_pending: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::MAX_PENDING_EVENTS;
    use std::collections::VecDeque;

    fn push_event_local(queue: &mut VecDeque<String>, event: String) {
        queue.push_back(event);
        while queue.len() > MAX_PENDING_EVENTS {
            let _ = queue.pop_front();
        }
    }

    #[test]
    fn event_queue_keeps_latest_events() {
        let mut queue = VecDeque::new();
        for index in 0..(MAX_PENDING_EVENTS + 5) {
            push_event_local(&mut queue, format!("event-{index}"));
        }

        assert_eq!(queue.len(), MAX_PENDING_EVENTS);
        assert_eq!(queue.front().map(String::as_str), Some("event-5"));
        let expected_last = format!("event-{}", MAX_PENDING_EVENTS + 4);
        assert_eq!(
            queue.back().map(String::as_str),
            Some(expected_last.as_str())
        );
    }
}
