//! Core-side dictation hotkey listener.
//!
//! Reads the `DictationConfig` from config, starts a module-owned global
//! hotkey listener on the core process, and broadcasts `dictation:toggle`
//! events over a `tokio::sync::broadcast` channel that the Socket.IO
//! bridge subscribes to — so the frontend receives hotkey presses without
//! any Tauri-side shortcut registration.

use once_cell::sync::Lazy;
use serde::Serialize;
use std::sync::Mutex;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
// macOS keeps the Tauri-owned global shortcut. Other platforms use TinyVoice's
// module-owned native listener through the same adapter as the voice server.
#[cfg(not(target_os = "macos"))]
use tinyvoice_bus::hotkey::{ActivationMode, HotkeyEvent};

const LOG_PREFIX: &str = "[dictation_listener]";

// ── Listener task handle (for stop support) ─────────────────────────

type ListenerTask = (CancellationToken, JoinHandle<()>);

static LISTENER_HANDLE: Lazy<Mutex<Option<ListenerTask>>> = Lazy::new(|| Mutex::new(None));

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
pub async fn start_if_enabled(ctx: &crate::core::runtime::DispatchCtx, config: &Config) {
    if !config.components.voice {
        log::info!("{LOG_PREFIX} [components].voice is off, skipping hotkey listener");
        return;
    }
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

    // Non-macOS: start the TinyVoice-owned native listener.
    #[cfg(not(target_os = "macos"))]
    start_module_listener(ctx, hotkey_str, config).await;
}

#[cfg(not(target_os = "macos"))]
async fn start_module_listener(
    ctx: &crate::core::runtime::DispatchCtx,
    hotkey_str: String,
    config: &Config,
) {
    // Map DictationActivationMode to our hotkey ActivationMode.
    let mode = match config.dictation.activation_mode {
        crate::config::DictationActivationMode::Push => ActivationMode::Push,
        crate::config::DictationActivationMode::Toggle => ActivationMode::Tap,
    };

    // Normalize the product spelling before passing it to TinyVoice.
    let normalized = normalize_hotkey(&hotkey_str);

    log::info!(
        "{LOG_PREFIX} starting dictation hotkey listener: hotkey={normalized} (raw={hotkey_str}) mode={mode:?}"
    );

    let cancel = CancellationToken::new();
    let (listener, mut hotkey_rx) =
        match crate::voice::server::hotkey_listener::start_hotkey_listener(
            ctx,
            config,
            &normalized,
            mode,
            &cancel,
        )
        .await
        {
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
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = task_cancel.cancelled() => break,
                event = hotkey_rx.recv() => match event {
                    Some(event) => event,
                    None => break,
                },
            };
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

        listener.stop().await;
        log::debug!("{LOG_PREFIX} hotkey listener stopped");
    });

    // Store handle so `stop()` can abort it on logout.
    if let Ok(mut guard) = LISTENER_HANDLE.lock() {
        *guard = Some((cancel, task));
    }
}

/// Stop the dictation hotkey listener if running.
///
/// Cancels the event forwarder and joins TinyVoice's native listener cleanup,
/// preventing duplicate hooks from accumulating across logout → login cycles.
pub async fn stop() {
    let listener = LISTENER_HANDLE
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());
    if let Some((cancel, handle)) = listener {
        cancel.cancel();
        if let Err(error) = handle.await {
            log::warn!("{LOG_PREFIX} dictation listener task ended: {error}");
        } else {
            log::info!("{LOG_PREFIX} dictation listener stopped");
        }
    }
}

/// Normalize the host-style hotkey spelling for TinyVoice's portable parser.
///
/// Converts `CmdOrCtrl+Shift+D` → `cmd+shift+d` (macOS) or `ctrl+shift+d` (other).
fn normalize_hotkey(hotkey: &str) -> String {
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
