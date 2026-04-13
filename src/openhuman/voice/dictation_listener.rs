//! Core-side dictation hotkey listener.
//!
//! Reads the `DictationConfig` from config, starts an `rdev`-based global
//! hotkey listener on the core process, and broadcasts `dictation:toggle`
//! events over a `tokio::sync::broadcast` channel that the Socket.IO
//! bridge subscribes to — so the frontend receives hotkey presses without
//! any Tauri-side shortcut registration.

use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::openhuman::config::Config;
use crate::openhuman::voice::hotkey::{self, ActivationMode, HotkeyEvent};

const LOG_PREFIX: &str = "[dictation_listener]";

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
pub fn publish_transcription(text: String) {
    log::debug!(
        "{LOG_PREFIX} publishing transcription: {} chars",
        text.len()
    );
    let _ = TRANSCRIPTION_BUS.send(text);
}

// ── Listener lifecycle ────────────────────────────────────────────────

/// Start the dictation hotkey listener if enabled in config.
///
/// Intended to be called once from `run_server()` as a background task.
/// Reads the `dictation` config section and registers the global hotkey.
/// When the hotkey fires, publishes a `DictationEvent` to the broadcast
/// channel that the Socket.IO bridge forwards to all connected clients.
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

    // Map DictationActivationMode to our hotkey ActivationMode.
    let mode = match config.dictation.activation_mode {
        crate::openhuman::config::DictationActivationMode::Push => ActivationMode::Push,
        crate::openhuman::config::DictationActivationMode::Toggle => ActivationMode::Tap,
    };

    // Normalize the hotkey string for rdev (CmdOrCtrl → cmd on macOS, ctrl on others).
    let normalized = normalize_hotkey_for_rdev(&hotkey_str);

    log::info!(
        "{LOG_PREFIX} starting dictation hotkey listener: hotkey={normalized} (raw={hotkey_str}) mode={mode:?}"
    );

    let combo = match hotkey::parse_hotkey(&normalized) {
        Ok(c) => c,
        Err(e) => {
            log::error!("{LOG_PREFIX} failed to parse hotkey '{normalized}': {e}");
            return;
        }
    };

    let (listener_handle, mut hotkey_rx) = match hotkey::start_listener(combo, mode) {
        Ok(pair) => pair,
        Err(e) => {
            log::error!("{LOG_PREFIX} failed to start hotkey listener: {e}");
            return;
        }
    };

    let mode_str = match mode {
        ActivationMode::Tap => "toggle",
        ActivationMode::Push => "push",
    };

    log::info!("{LOG_PREFIX} dictation hotkey active: {normalized}");

    // Forward hotkey events to the broadcast channel.
    tokio::spawn(async move {
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
mod tests {
    use super::*;

    #[test]
    fn normalize_cmdorctrl_macos() {
        let result = normalize_hotkey_for_rdev("CmdOrCtrl+Shift+D");
        if cfg!(target_os = "macos") {
            assert_eq!(result, "cmd+shift+d");
        } else {
            assert_eq!(result, "ctrl+shift+d");
        }
    }

    #[test]
    fn normalize_plain_keys() {
        assert_eq!(normalize_hotkey_for_rdev("Ctrl+Space"), "ctrl+space");
    }

    #[test]
    fn normalize_preserves_structure() {
        assert_eq!(normalize_hotkey_for_rdev("Alt+Shift+F5"), "alt+shift+f5");
    }

    #[test]
    fn subscribe_returns_receiver() {
        let _rx = subscribe_dictation_events();
    }

    #[test]
    fn publish_dictation_event_reaches_subscriber() {
        let mut rx = subscribe_dictation_events();
        publish_dictation_event(DictationEvent {
            event_type: "pressed".to_string(),
            hotkey: "chat_button".to_string(),
            activation_mode: "toggle".to_string(),
        });
        let evt = rx.try_recv().expect("should receive dictation event");
        assert_eq!(evt.event_type, "pressed");
        assert_eq!(evt.hotkey, "chat_button");
    }

    #[test]
    fn publish_transcription_reaches_subscriber() {
        let mut rx = subscribe_transcription_results();
        publish_transcription("hello world".to_string());
        let text = rx.try_recv().expect("should receive transcription");
        assert_eq!(text, "hello world");
    }

    #[test]
    fn normalize_commandorcontrol_alias() {
        let result = normalize_hotkey_for_rdev("CommandOrControl+Alt+K");
        if cfg!(target_os = "macos") {
            assert_eq!(result, "cmd+alt+k");
        } else {
            assert_eq!(result, "ctrl+alt+k");
        }
    }

    #[test]
    fn dictation_event_serializes_wire_type_field() {
        let evt = DictationEvent {
            event_type: "released".to_string(),
            hotkey: "fn".to_string(),
            activation_mode: "push".to_string(),
        };
        let json = serde_json::to_value(evt).expect("serialize dictation event");
        assert_eq!(json["type"], "released");
        assert_eq!(json["hotkey"], "fn");
        assert_eq!(json["activation_mode"], "push");
    }
}
