//! The voice server runtime: owns the hotkey event loop that turns
//! press/release events into recordings, and hands each finished recording
//! off to the background pipeline.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, info, warn};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::Config;
use crate::voice::audio_capture::{self, RecordingHandle};
use tinyvoice_bus::hotkey::{ActivationMode, HotkeyEvent};

use super::hotkey_listener::start_hotkey_listener;
use super::pipeline::{capture_expected_app_name, process_recording_bg};
use super::types::{ServerState, VoiceServerConfig, VoiceServerStatus};
use super::LOG_PREFIX;

/// The voice server runtime.
pub struct VoiceServer {
    state: Arc<Mutex<ServerState>>,
    /// Wrapped in a Mutex so `run()` can replace it with a fresh token after
    /// `stop()` — a `CancellationToken` cannot be un-cancelled.
    cancel: Mutex<CancellationToken>,
    config: VoiceServerConfig,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    session_generation: Arc<std::sync::atomic::AtomicU64>,
    last_error: Arc<Mutex<Option<String>>>,
    /// Rolling buffer of recent transcriptions used as STT context for
    /// better continuity across consecutive recordings.
    recent_transcripts: Arc<Mutex<Vec<String>>>,
}

impl VoiceServer {
    pub fn new(config: VoiceServerConfig) -> Self {
        Self {
            state: Arc::new(Mutex::new(ServerState::Stopped)),
            cancel: Mutex::new(CancellationToken::new()),
            config,
            transcription_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_error: Arc::new(Mutex::new(None)),
            recent_transcripts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get the current server status.
    pub async fn status(&self) -> VoiceServerStatus {
        VoiceServerStatus {
            state: *self.state.lock().await,
            hotkey: self.config.hotkey.clone(),
            activation_mode: self.config.activation_mode,
            transcription_count: self.transcription_count.load(Ordering::Relaxed),
            last_error: self.last_error.lock().await.clone(),
        }
    }

    /// Run the voice server. Blocks until stopped.
    ///
    /// This is the main entry point for both embedded and standalone modes.
    pub async fn run(&self, app_config: &Config) -> Result<(), String> {
        // Atomically transition Stopped → Idle to prevent concurrent run() calls.
        // The globe listener compilation can take several seconds; without this
        // guard the RPC handler sees "Stopped" and spawns a duplicate run().
        //
        // Also replace the cancellation token with a fresh one — a cancelled
        // token cannot be reused (stop() cancels it permanently).
        let cancel = {
            // Lock cancel FIRST, then state — same order as stop() — to
            // prevent a race where stop() cancels the old token between
            // setting Idle and swapping the token.
            let mut cancel_guard = self.cancel.lock().await;
            let mut state = self.state.lock().await;
            if *state != ServerState::Stopped {
                return Err(format!("voice server already running (state={:?})", *state));
            }

            let fresh = CancellationToken::new();
            *cancel_guard = fresh.clone();
            *state = ServerState::Idle;
            fresh
        };

        info!(
            "{LOG_PREFIX} starting voice server: hotkey={} mode={:?}",
            self.config.hotkey, self.config.activation_mode
        );

        // On macOS, the Fn/Globe key is intercepted by the system before
        // rdev's CGEventTap can see it. Use the Swift-based globe listener
        // instead, which monitors NSEvent.flagsChanged for the .function flag.
        let (listener_handle, mut hotkey_rx) = match start_hotkey_listener(
            app_config,
            &self.config.hotkey,
            self.config.activation_mode,
            &cancel,
        )
        .await
        {
            Ok(pair) => pair,
            Err(e) => {
                *self.state.lock().await = ServerState::Stopped;
                return Err(e);
            }
        };

        info!("{LOG_PREFIX} voice server ready, listening for hotkey");

        let mut recording: Option<RecordingHandle> = None;
        let mut recording_expected_app: Option<String> = None;

        // Pending recording setup: `start_recording()` runs on a blocking
        // thread so the event loop stays responsive to Release events that
        // macOS fires almost immediately for the Fn key.
        let mut recording_pending_rx: Option<
            tokio::sync::oneshot::Receiver<Result<(RecordingHandle, Option<String>), String>>,
        > = None;
        let mut pending_generation: Option<u64> = None;
        let mut recording_generation: Option<u64> = None;
        // Set when a stop-intent event (Release/Pressed toggle) arrives before
        // recording has started.
        let mut pending_stop = false;
        // Deferred stop deadline used when stop intent arrives during setup.
        // Keeping this in a select! branch avoids blocking the hotkey loop.
        let mut deferred_stop_deadline: Option<tokio::time::Instant> = None;

        /// Minimum recording duration after setup completes. If the user
        /// released the hotkey while cpal was still initialising, we keep
        /// recording for at least this long to capture actual speech.
        const MIN_RECORDING_AFTER_SETUP: Duration = Duration::from_millis(1500);

        loop {
            // Build a future that resolves when the pending recording setup
            // completes, or never if there is no pending setup.
            let pending_ready = async {
                match recording_pending_rx.as_mut() {
                    Some(rx) => rx.await,
                    None => std::future::pending().await,
                }
            };
            let deferred_stop_ready = async {
                match deferred_stop_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            };

            tokio::select! {
                ev = hotkey_rx.recv() => {
                    let event = match ev {
                        Some(e) => e,
                        None => {
                            warn!("{LOG_PREFIX} hotkey channel closed");
                            break;
                        }
                    };

                    // Forward hotkey event to the dictation bus so Socket.IO
                    // clients receive dictation:toggle events even when the
                    // dictation_listener is not running (single TinyVoice listener).
                    {
                        use crate::voice::dictation_listener;
                        let event_type = match event {
                            HotkeyEvent::Pressed => "pressed",
                            HotkeyEvent::Released => "released",
                        };
                        dictation_listener::publish_dictation_event(
                            dictation_listener::DictationEvent {
                                event_type: event_type.to_string(),
                                hotkey: self.config.hotkey.clone(),
                                activation_mode: match self.config.activation_mode {
                                    ActivationMode::Tap => "toggle".to_string(),
                                    ActivationMode::Push => "push".to_string(),
                                },
                            },
                        );
                    }

                    match event {
                        HotkeyEvent::Pressed => {
                            let current_state = *self.state.lock().await;
                            info!(
                                "{LOG_PREFIX} received hotkey event=Pressed state_before={current_state:?} recording={} pending={}",
                                recording.is_some(),
                                recording_pending_rx.is_some()
                            );
                            if recording.is_some() {
                                // Recording in progress → stop it (tap toggle or
                                // unreliable-release keys like Fn that always send Pressed).
                                debug!("{LOG_PREFIX} hotkey pressed while recording → stopping");
                                deferred_stop_deadline = None;
                                if let Some(handle) = recording.take() {
                                    self.spawn_process_recording(
                                        handle,
                                        app_config,
                                        recording_generation.take().unwrap_or_default(),
                                        recording_expected_app.take(),
                                    );
                                }
                            } else if recording_pending_rx.is_some() {
                                info!("{LOG_PREFIX} hotkey pressed while recording setup pending — buffering stop intent");
                                pending_stop = true;
                            } else {
                                let generation =
                                    self.session_generation.fetch_add(1, Ordering::Relaxed) + 1;
                                debug!("{LOG_PREFIX} hotkey pressed → starting recording (non-blocking)");
                                debug!(
                                    "{LOG_PREFIX} assigned recording generation={} for new session",
                                    generation
                                );

                                // Start recording on a blocking thread so the
                                // event loop remains responsive to Release.
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                let capture_config = app_config.clone();
                                crate::core::runtime::spawn_scoped(async move {
                                    let expected_app = capture_expected_app_name(&capture_config).await;
                                    let result = audio_capture::start_recording(&capture_config).await
                                        .map(|handle| (handle, expected_app));
                                    let _ = tx.send(result);
                                });
                                recording_pending_rx = Some(rx);
                                pending_generation = Some(generation);
                                pending_stop = false;
                                deferred_stop_deadline = None;
                                *self.state.lock().await = ServerState::Recording;
                            }
                        }
                        HotkeyEvent::Released => {
                            info!(
                                "{LOG_PREFIX} received hotkey event=Released recording={} pending={}",
                                recording.is_some(),
                                recording_pending_rx.is_some()
                            );
                            if let Some(handle) = recording.take() {
                                debug!("{LOG_PREFIX} hotkey released → stopping recording");
                                deferred_stop_deadline = None;
                                self.spawn_process_recording(
                                    handle,
                                    app_config,
                                    recording_generation.take().unwrap_or_default(),
                                    recording_expected_app.take(),
                                );
                            } else if recording_pending_rx.is_some() {
                                // Release arrived before recording setup finished.
                                // Buffer stop intent — we'll handle it once the handle arrives.
                                info!("{LOG_PREFIX} release buffered — recording setup still pending");
                                pending_stop = true;
                            } else {
                                debug!("{LOG_PREFIX} release received with no active recording (normal for unreliable-release keys)");
                            }
                        }
                    }
                }

                result = pending_ready => {
                    // Recording setup completed (or failed).
                    recording_pending_rx = None;
                    match result {
                        Ok(Ok((handle, expected_app))) => {
                            // Check for a buffered stop event that lost the
                            // select! race against pending_ready. On warm CPAL
                            // init both branches may be ready simultaneously;
                            // select! picks one pseudo-randomly, so a Released
                            // event can sit unprocessed in hotkey_rx.
                            let had_pending_stop = pending_stop;
                            if !pending_stop {
                                if let Ok(buffered) = hotkey_rx.try_recv() {
                                    match buffered {
                                        HotkeyEvent::Released => {
                                            info!(
                                                "{LOG_PREFIX} recording handle ready — found buffered Released in hotkey_rx (select! race recovered)"
                                            );
                                            pending_stop = true;
                                        }
                                        HotkeyEvent::Pressed => {
                                            // A second Pressed while pending means
                                            // user wants to stop (tap-style). Treat
                                            // the same as a stop intent.
                                            info!(
                                                "{LOG_PREFIX} recording handle ready — found buffered Pressed in hotkey_rx (treating as stop intent)"
                                            );
                                            pending_stop = true;
                                        }
                                    }
                                }
                            }

                            info!(
                                "{LOG_PREFIX} recording handle ready (pending_stop={pending_stop}, was_buffered={})",
                                !had_pending_stop && pending_stop
                            );

                            if pending_stop {
                                // A stop intent arrived while cpal was initialising.
                                // Keep recording for a minimum duration, then stop
                                // via non-blocking deferred deadline branch.
                                pending_stop = false;
                                recording = Some(handle);
                                recording_generation = pending_generation.take();
                                recording_expected_app = expected_app;

                                info!(
                                    "{LOG_PREFIX} deferred stop: recording for at least {}ms",
                                    MIN_RECORDING_AFTER_SETUP.as_millis()
                                );
                                deferred_stop_deadline = Some(
                                    tokio::time::Instant::now() + MIN_RECORDING_AFTER_SETUP,
                                );
                            } else {
                                recording = Some(handle);
                                recording_generation = pending_generation.take();
                                recording_expected_app = expected_app;
                                deferred_stop_deadline = None;

                                info!("{LOG_PREFIX} recording started (live)");
                            }
                        }
                        Ok(Err(e)) => {
                            pending_stop = false;
                            deferred_stop_deadline = None;
                            pending_generation = None;
                            warn!("{LOG_PREFIX} failed to start recording: {e}");
                            *self.state.lock().await = ServerState::Idle;
                            *self.last_error.lock().await = Some(e);
                        }
                        Err(_) => {
                            pending_stop = false;
                            deferred_stop_deadline = None;
                            pending_generation = None;
                            warn!("{LOG_PREFIX} recording setup task dropped");
                            *self.state.lock().await = ServerState::Idle;
                        }
                    }
                }

                _ = deferred_stop_ready => {
                    deferred_stop_deadline = None;
                    if let Some(handle) = recording.take() {
                        info!(
                            "{LOG_PREFIX} deferred stop deadline reached after {}ms, stopping recording",
                            MIN_RECORDING_AFTER_SETUP.as_millis()
                        );
                        self.spawn_process_recording(
                            handle,
                            app_config,
                            recording_generation.take().unwrap_or_default(),
                            recording_expected_app.take(),
                        );
                    }
                }

                _ = cancel.cancelled() => {
                    debug!("{LOG_PREFIX} cancellation received");
                    break;
                }
            }
        }

        listener_handle.stop().await;
        *self.state.lock().await = ServerState::Stopped;
        info!("{LOG_PREFIX} voice server stopped");

        Ok(())
    }

    /// Stop the voice server and wait for it to reach `Stopped` state.
    ///
    /// Cancels the run-loop token and polls until the state transitions to
    /// `Stopped` (or a 5-second timeout expires). This prevents a fast
    /// logout → login cycle from seeing a stale `Idle`/`Recording` state
    /// and skipping the restart.
    pub async fn stop(&self) {
        info!("{LOG_PREFIX} stopping voice server");
        self.cancel.lock().await.cancel();

        // Wait for the run-loop to observe cancellation and set Stopped.
        for _ in 0..50 {
            if *self.state.lock().await == ServerState::Stopped {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        warn!("{LOG_PREFIX} stop timed out after 5s — state may not be Stopped");
    }

    /// Record an error message so it can be surfaced via status().
    pub async fn set_last_error(&self, msg: &str) {
        *self.last_error.lock().await = Some(msg.to_string());
    }

    /// Spawn `process_recording` as a background task so the hotkey event
    /// loop is not blocked during transcription. This ensures rapid
    /// consecutive Fn presses are never missed.
    fn spawn_process_recording(
        &self,
        handle: RecordingHandle,
        config: &Config,
        generation: u64,
        expected_app: Option<String>,
    ) {
        let pipeline_id = Uuid::new_v4().to_string()[..8].to_string();
        let state = self.state.clone();
        let server_config = self.config.clone();
        let transcription_count = self.transcription_count.clone();
        let session_generation = self.session_generation.clone();
        let last_error = self.last_error.clone();
        let recent_transcripts = self.recent_transcripts.clone();
        let app_config = config.clone();

        info!(
            "{LOG_PREFIX} [pipeline={pipeline_id}] spawning process_recording (generation={generation})"
        );

        crate::core::runtime::spawn_scoped(async move {
            process_recording_bg(
                &pipeline_id,
                handle,
                &app_config,
                &server_config,
                state,
                transcription_count,
                session_generation,
                generation,
                last_error,
                recent_transcripts,
                expected_app,
            )
            .await;
        });
    }
}
