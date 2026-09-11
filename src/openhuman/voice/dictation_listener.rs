//! Core-side dictation hotkey listener.
//!
//! Reads the `DictationConfig` from config, starts an `rdev`-based global
//! hotkey listener on the core process, and broadcasts `dictation:toggle`
//! events over a `tokio::sync::broadcast` channel that the Socket.IO
//! bridge subscribes to — so the frontend receives hotkey presses without
//! any Tauri-side shortcut registration.

use once_cell::sync::Lazy;
use serde::Serialize;
use std::sync::Mutex;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::openhuman::config::Config;
// The rdev-based listener (non-macOS) resolves the hotkey combo + activation mode
// against the cross-platform `hotkey` module. macOS uses a separate path and
// never compiles `start_rdev_listener`, so gate the import to avoid an unused
// import warning there.
#[cfg(not(target_os = "macos"))]
use super::hotkey::{self, ActivationMode, HotkeyEvent};

const LOG_PREFIX: &str = "[dictation_listener]";

// ── Listener task handle (for stop support) ─────────────────────────

static LISTENER_HANDLE: Lazy<Mutex<Option<JoinHandle<()>>>> = Lazy::new(|| Mutex::new(None));

// ── Broadcast channel for dictation events ────────────────────────────

/// A dictation event broadcast to Socket.IO clients.
#[derive(Debug, Clone, Serialize)]
pub struct DictationEvent {
    /// Event type: `"pressed"` or `"released"`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The hotkey that triggered this event.
    pub hotkey: String,
    /// The activation mode in use.
    pub activation_mode: String,
}

static DICTATION_BUS: Lazy<broadcast::Sender<DictationEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(64);
    tx
});

/// Subscribe to dictation events (used by the Socket.IO bridge).
pub fn subscribe_dictation_events() -> broadcast::Receiver<DictationEvent> {
    DICTATION_BUS.subscribe()
}

pub fn publish_dictation_event(event: DictationEvent) {
    let _ = DICTATION_BUS.send(event);
}

// ── Transcription result broadcast ───────────────────────────────────

static TRANSCRIPTION_BUS: Lazy<broadcast::Sender<String>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(64);
    tx
});

/// Subscribe to transcription results (used by the Socket.IO bridge).
pub fn subscribe_transcription_results() -> broadcast::Receiver<String> {
    TRANSCRIPTION_BUS.subscribe()
}

/// Broadcast a completed transcription to frontend clients.
///
/// Returns the number of receivers that received the message, or 0 if
/// there are no active subscribers.
pub fn publish_transcription(text: String) -> usize {
    let receiver_count = TRANSCRIPTION_BUS.receiver_count();
    log::info!(
        "{LOG_PREFIX} publishing transcription: {} chars, {} active receivers",
        text.len(),
        receiver_count
    );
    match TRANSCRIPTION_BUS.send(text) {
        Ok(n) => {
            log::debug!("{LOG_PREFIX} transcription delivered to {n} receivers");
            n
        }
        Err(_) => {
            log::warn!("{LOG_PREFIX} transcription send failed — no active receivers");
            0
        }
    }
}

// ── Listener lifecycle ────────────────────────────────────────────────

/// Start the dictation hotkey listener if enabled in config.
///
/// Intended to be called once from `run_server()` as a background task.
/// Reads the `dictation` config section and registers the global hotkey.
/// When the hotkey fires, publishes a `DictationEvent` to the broadcast
/// channel that the Socket.IO bridge forwards to all connected clients.
///
/// **macOS note**: this function is a no-op on macOS. Starting with macOS 26,
/// `TSMGetInputSourceProperty` is enforced to run on the main dispatch queue;
/// rdev's CGEventTap callback fires on a background thread and crashes with
/// `EXC_BREAKPOINT` (`dispatch_assert_queue_fail`) on the first key press. The
/// Tauri host already registers the shortcut via `tauri-plugin-global-shortcut`
/// (main-thread-safe) and emits `dictation://toggle` to the frontend, making
/// the core-side rdev listener redundant on macOS. (#2677)
pub async fn start_if_enabled(config: &Config) {
    if !config.dictation.enabled {
        log::info!("{LOG_PREFIX} dictation disabled in config, skipping hotkey listener");
        return;
    }

    let hotkey_str = config.dictation.hotkey.clone();
    if hotkey_str.is_empty() {
        log::warn!("{LOG_PREFIX} dictation enabled but no hotkey configured");
        return;
    }

    // On macOS the rdev listener must not start: rdev's CGEventTap callback
    // calls TSMGetInputSourceProperty off the main thread, crashing with
    // EXC_BREAKPOINT on macOS 26 (dispatch_assert_queue_fail). The Tauri host
    // handles this shortcut via tauri-plugin-global-shortcut instead. (#2677)
    #[cfg(target_os = "macos")]
    {
        log::info!(
            "{LOG_PREFIX} macOS: skipping rdev hotkey listener — \
             handled by Tauri host via tauri-plugin-global-shortcut (issue #2677)"
        );
    }

    // Non-macOS: start the rdev-based listener.
    #[cfg(not(target_os = "macos"))]
    start_rdev_listener(hotkey_str, config).await;
}

#[cfg(not(target_os = "macos"))]
async fn start_rdev_listener(hotkey_str: String, config: &Config) {
    // Map DictationActivationMode to our hotkey ActivationMode.
    let mode = match config.dictation.activation_mode {
        crate::openhuman::config::DictationActivationMode::Push => ActivationMode::Push,
        crate::openhuman::config::DictationActivationMode::Toggle => ActivationMode::Tap,
    };

    // Normalize the hotkey string for rdev (CmdOrCtrl → ctrl on non-macOS).
    let normalized = normalize_hotkey_for_rdev(&hotkey_str);

    log::info!(
        "{LOG_PREFIX} starting dictation hotkey listener: hotkey={normalized} (raw={hotkey_str}) mode={mode:?}"
    );

    let combo = match hotkey::parse_hotkey(&normalized) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("{LOG_PREFIX} failed to parse hotkey '{normalized}': {e}");
            return;
        }
    };

    let (listener_handle, mut hotkey_rx) = match hotkey::start_listener(combo, mode) {
        Ok(pair) => pair,
        Err(e) => {
            log::warn!("{LOG_PREFIX} failed to start hotkey listener: {e}");
            return;
        }
    };

    let mode_str = match mode {
        ActivationMode::Tap => "toggle",
        ActivationMode::Push => "push",
    };

    log::info!("{LOG_PREFIX} dictation hotkey active: {normalized}");

    // Forward hotkey events to the broadcast channel.
    let task = tokio::spawn(async move {
        // Keep the listener handle alive for the lifetime of this task.
        let _handle = listener_handle;

        while let Some(event) = hotkey_rx.recv().await {
            let event_type = match event {
                HotkeyEvent::Pressed => "pressed",
                HotkeyEvent::Released => "released",
            };

            log::debug!("{LOG_PREFIX} hotkey {event_type}");

            publish_dictation_event(DictationEvent {
                event_type: event_type.to_string(),
                hotkey: normalized.clone(),
                activation_mode: mode_str.to_string(),
            });
        }

        log::warn!("{LOG_PREFIX} hotkey event channel closed, listener stopping");
    });

    // Store handle so `stop()` can abort it on logout.
    if let Ok(mut guard) = LISTENER_HANDLE.lock() {
        *guard = Some(task);
    }
}

/// Stop the dictation hotkey listener if running.
///
/// Aborts the spawned forwarder task and drops the `rdev` listener handle,
/// preventing duplicate hotkey listeners from accumulating across
/// logout → login cycles.
pub fn stop() {
    if let Ok(mut guard) = LISTENER_HANDLE.lock() {
        if let Some(handle) = guard.take() {
            handle.abort();
            log::info!("{LOG_PREFIX} dictation listener stopped");
        }
    }
}

/// Normalize a Tauri-style hotkey string to rdev-compatible format.
///
/// Converts `CmdOrCtrl+Shift+D` → `cmd+shift+d` (macOS) or `ctrl+shift+d` (other).
fn normalize_hotkey_for_rdev(hotkey: &str) -> String {
    let parts: Vec<&str> = hotkey.split('+').map(|s| s.trim()).collect();
    let mut result = Vec::new();

    for part in parts {
        let lower = part.to_lowercase();
        let mapped = match lower.as_str() {
            "cmdorctrl" | "commandorcontrol" => {
                if cfg!(target_os = "macos") {
                    "cmd"
                } else {
                    "ctrl"
                }
            }
            "cmd" | "command" => "cmd",
            "ctrl" | "control" => "ctrl",
            other => other,
        };
        result.push(mapped.to_string());
    }

    result.join("+")
}

#[cfg(test)]
#[path = "dictation_listener_tests.rs"]
mod tests;
