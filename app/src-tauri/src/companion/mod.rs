//! Desktop companion — Clicky-style interaction loop (shell-side).
//!
//! Migrated out of the Rust core (`src/openhuman/desktop_companion`) into the
//! Tauri shell. Ties hotkey activation, **native** microphone capture, LLM
//! reasoning, and speech synthesis into one product experience:
//!
//! - `audio`    — native `cpal` mic capture (mono i16 PCM ~16 kHz)
//! - `session`  — session lifecycle + state machine (idle→listening→…→idle)
//! - `pipeline` — one interaction turn (STT → LLM → TTS)
//! - `events`   — `companion://state_changed` Tauri event delivery
//!
//! STT/TTS run in the embedded core over JSON-RPC; the LLM turn runs shell-side
//! against the backend using creds fetched from core. The five Tauri commands
//! below mirror the five RPC methods the old core `desktop_companion` exposed.

pub mod audio;
pub mod events;
pub mod pipeline;
pub mod session;
pub mod types;

use log::{debug, info, warn};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use crate::AppRuntime;
use audio::MicCapture;
use types::*;

const LOG_PREFIX: &str = "[companion]";

/// Process-global companion configuration (persisted only in-memory for now —
/// mirrors the old core stub, but round-trips get/set instead of erroring).
static CONFIG: Mutex<Option<CompanionConfig>> = Mutex::new(None);

/// The in-flight mic capture, if the companion is currently Listening. Used to
/// implement both tap-to-talk and push-to-talk activation.
static ACTIVE_CAPTURE: Mutex<Option<MicCapture>> = Mutex::new(None);

/// Cancellation handle for the currently running STT/LLM/TTS turn.
///
/// A new capture cancels the previous turn before entering `Listening`, so
/// stale turn cleanup cannot race the interrupting utterance. The monotonic id
/// lets a completed old task avoid clearing a newer turn's token.
struct ActiveTurn {
    id: u64,
    cancel: CancellationToken,
}

static ACTIVE_TURN: Mutex<Option<ActiveTurn>> = Mutex::new(None);
static NEXT_TURN_ID: AtomicU64 = AtomicU64::new(1);
static SESSION_LIFECYCLE: Mutex<()> = Mutex::new(());

/// Install the app handle so the session/pipeline can emit frontend events from
/// outside a command context. Call once from `Builder::setup`.
pub fn setup(app: &AppHandle<AppRuntime>) {
    events::set_app_handle(app.clone());
    info!("{LOG_PREFIX} setup complete");
}

pub(super) fn current_config() -> CompanionConfig {
    CONFIG.lock().clone().unwrap_or_default()
}

// ── Tauri commands (mirror the 5 former core RPC methods) ──────────────

/// Start a desktop companion session with explicit consent.
#[tauri::command]
pub(crate) async fn companion_start_session(
    app: AppHandle<AppRuntime>,
    consent: bool,
) -> Result<StartCompanionSessionResult, String> {
    debug!("{LOG_PREFIX} companion_start_session consent={consent}");
    let _lifecycle = SESSION_LIFECYCLE.lock();
    cleanup_expired_session(&app);
    let ttl_secs = current_config().ttl_secs;
    session::start_session(&StartCompanionSessionParams {
        consent,
        ttl_secs: Some(ttl_secs),
    })
}

/// Stop the active desktop companion session.
#[tauri::command]
pub(crate) async fn companion_stop_session(
    app: AppHandle<AppRuntime>,
) -> Result<StopCompanionSessionResult, String> {
    debug!("{LOG_PREFIX} companion_stop_session");
    let _lifecycle = SESSION_LIFECYCLE.lock();
    stop_session_resources();
    let result = session::stop_session(&StopCompanionSessionParams {
        reason: Some("user_requested".into()),
    })?;
    crate::companion_commands::unregister_companion_hotkey_for_app(&app)?;
    Ok(result)
}

fn stop_session_resources() {
    cancel_active_turn();
    if let Some(capture) = ACTIVE_CAPTURE.lock().take() {
        let _ = capture.stop();
    }
}

fn cleanup_expired_session(app: &AppHandle<AppRuntime>) {
    if session::session_is_expired() {
        stop_session_resources();
        let _ = session::expire_session_if_needed();
        if let Err(error) = crate::companion_commands::unregister_companion_hotkey_for_app(app) {
            warn!("{LOG_PREFIX} failed to unregister expired session hotkey: {error}");
        }
    }
}

/// Get the current desktop companion session status.
#[tauri::command]
pub(crate) async fn companion_status(
    app: AppHandle<AppRuntime>,
) -> Result<CompanionSessionStatus, String> {
    let _lifecycle = SESSION_LIFECYCLE.lock();
    cleanup_expired_session(&app);
    Ok(session::session_status())
}

/// Get the current desktop companion configuration.
#[tauri::command]
pub(crate) async fn companion_config_get() -> Result<CompanionConfig, String> {
    Ok(current_config())
}

/// Update the desktop companion configuration.
#[tauri::command]
pub(crate) async fn companion_config_set(
    config: CompanionConfig,
) -> Result<CompanionConfig, String> {
    debug!(
        "{LOG_PREFIX} companion_config_set hotkey={} mode={}",
        config.hotkey, config.activation_mode
    );
    *CONFIG.lock() = Some(config.clone());
    Ok(config)
}

// ── Activation driven by the hotkey / companion_activate ──────────────

/// Handle a global-hotkey press according to the configured activation mode.
///
/// Push mode begins capture on press; tap mode toggles capture on each press.
pub fn handle_hotkey_pressed(app: AppHandle<AppRuntime>) {
    if current_config()
        .activation_mode
        .eq_ignore_ascii_case("push")
    {
        start_capture(&app);
    } else {
        handle_activation(app);
    }
}

/// Handle a global-hotkey release according to the configured activation mode.
///
/// Only push mode consumes releases. Tap mode intentionally waits for the next
/// press, preserving toggle behavior.
pub fn handle_hotkey_released(_app: AppHandle<AppRuntime>) {
    if current_config()
        .activation_mode
        .eq_ignore_ascii_case("push")
    {
        finish_capture_and_run();
    }
}

/// Handle a companion activation (hotkey press or the `companion_activate`
/// command). Tap-to-talk: the first activation while a session is active starts
/// native mic capture (Listening); the next activation stops capture and spawns
/// the interaction turn.
pub fn handle_activation(app: AppHandle<AppRuntime>) {
    if ACTIVE_CAPTURE.lock().is_some() {
        finish_capture_and_run();
    } else {
        start_capture(&app);
    }
}

fn start_capture(app: &AppHandle<AppRuntime>) {
    let _lifecycle = SESSION_LIFECYCLE.lock();
    cleanup_expired_session(app);
    let status = session::session_status();
    if !status.active {
        debug!("{LOG_PREFIX} activation ignored — no active session");
        return;
    }

    let mut cap = ACTIVE_CAPTURE.lock();
    if cap.is_some() {
        debug!("{LOG_PREFIX} capture start ignored — already listening");
        return;
    }

    // Interrupt any previous STT/LLM/TTS task before the new capture owns the
    // Listening state. The pipeline observes this token between every phase.
    cancel_active_turn();
    // A cancelled pre-STT turn can still own Listening after its capture was
    // consumed. Retire that state while holding the capture lock so its later
    // cleanup cannot race a newly-started microphone capture.
    session::finish_listening_turn();

    if let Err(e) = session::transition_state(CompanionState::Listening, None) {
        warn!("{LOG_PREFIX} could not enter Listening: {e}");
        return;
    }
    match MicCapture::start() {
        Ok(c) => {
            *cap = Some(c);
            info!("{LOG_PREFIX} listening — mic capture started");
        }
        Err(e) => {
            warn!("{LOG_PREFIX} mic capture failed to start: {e}");
            session::recover_after_turn_error(format!("mic capture failed: {e}"));
        }
    }
}

fn finish_capture_and_run() {
    let _lifecycle = SESSION_LIFECYCLE.lock();
    let Some(capture) = ACTIVE_CAPTURE.lock().take() else {
        debug!("{LOG_PREFIX} capture stop ignored — not listening");
        return;
    };
    let (samples, rate) = capture.stop();
    info!("{LOG_PREFIX} captured {} samples @ {rate}Hz", samples.len());
    let (turn_id, cancel) = register_active_turn();
    tauri::async_runtime::spawn(async move {
        let result = pipeline::run_audio_turn(&samples, rate, cancel.clone()).await;
        if let Err(e) = result {
            if cancel.is_cancelled() {
                debug!("{LOG_PREFIX} interrupted companion turn stopped: {e}");
            } else {
                warn!("{LOG_PREFIX} companion turn failed: {e}");
                session::recover_after_turn_error(e);
            }
        }
        clear_active_turn(turn_id);
    });
}

pub(super) fn finish_listening_without_active_capture() {
    let capture = ACTIVE_CAPTURE.lock();
    if capture.is_none() {
        session::finish_listening_turn();
    }
}

fn register_active_turn() -> (u64, CancellationToken) {
    let id = NEXT_TURN_ID.fetch_add(1, Ordering::Relaxed);
    let cancel = CancellationToken::new();
    let mut active = ACTIVE_TURN.lock();
    if let Some(previous) = active.take() {
        previous.cancel.cancel();
    }
    *active = Some(ActiveTurn {
        id,
        cancel: cancel.clone(),
    });
    (id, cancel)
}

fn cancel_active_turn() {
    if let Some(active) = ACTIVE_TURN.lock().take() {
        active.cancel.cancel();
    }
}

fn clear_active_turn(id: u64) {
    let mut active = ACTIVE_TURN.lock();
    if active.as_ref().map(|turn| turn.id) == Some(id) {
        active.take();
    }
}

#[cfg(test)]
mod turn_cancellation_tests {
    use super::*;

    #[test]
    fn interrupt_cancels_in_flight_turn_and_old_completion_cannot_clear_new_token() {
        cancel_active_turn();

        let (old_id, old_token) = register_active_turn();
        cancel_active_turn();
        assert!(old_token.is_cancelled());

        let (new_id, new_token) = register_active_turn();
        clear_active_turn(old_id);
        assert!(!new_token.is_cancelled());
        assert_eq!(
            ACTIVE_TURN.lock().as_ref().map(|turn| turn.id),
            Some(new_id)
        );

        cancel_active_turn();
        assert!(new_token.is_cancelled());
    }
}
