//! The voice server runtime: owns the hotkey event loop that turns
//! press/release events into recordings, and hands each finished recording
//! off to the background pipeline. The state machine deciding what each event
//! means lives in the `tinyvoice` module; this loop reports facts to it
//! (`SessionAdvance`) and performs the action it answers with.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use log::{debug, info, warn};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::Config;
use crate::modules::voice::{RecorderSession, SessionAction, SessionInput};
use crate::voice::audio_capture::{self, RecordingHandle};
use tinyvoice_bus::hotkey::{ActivationMode, HotkeyEvent};

use super::hotkey_listener::start_hotkey_listener;
use super::pipeline::{capture_expected_app_name, process_recording_bg};
use super::recorder::Recorder;
use super::types::{ServerState, VoiceServerConfig, VoiceServerStatus};
use super::LOG_PREFIX;

type RecordingSetupReceiver =
    tokio::sync::oneshot::Receiver<Result<(RecordingHandle, Option<String>), String>>;

/// The voice server runtime.
pub struct VoiceServer {
    /// Mirror of the module-owned session phase; see [`Recorder`].
    state: Arc<Mutex<ServerState>>,
    /// The live session, present only while `run()` is active.
    recorder: Mutex<Option<Recorder>>,
    /// Wrapped in a Mutex so `run()` can replace it with a fresh token after
    /// `stop()` — a `CancellationToken` cannot be un-cancelled.
    cancel: Mutex<CancellationToken>,
    config: VoiceServerConfig,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    last_error: Arc<Mutex<Option<String>>>,
    /// Rolling buffer of recent transcriptions used as STT context for
    /// better continuity across consecutive recordings.
    recent_transcripts: Arc<Mutex<Vec<String>>>,
}

impl VoiceServer {
    pub fn new(config: VoiceServerConfig) -> Self {
        Self {
            state: Arc::new(Mutex::new(ServerState::Stopped)),
            recorder: Mutex::new(None),
            cancel: Mutex::new(CancellationToken::new()),
            config,
            transcription_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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
    pub async fn run(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        app_config: &Config,
    ) -> Result<(), String> {
        // Atomically transition Stopped → Idle to prevent concurrent run() calls.
        // The globe listener compilation can take several seconds; without this
        // guard the RPC handler sees "Stopped" and spawns a duplicate run().
        //
        // Also replace the cancellation token with a fresh one — a cancelled
        // token cannot be reused (stop() cancels it permanently).
        let (cancel, recorder) = {
            // Lock cancel FIRST, then state — same order as stop() — to
            // prevent a race where stop() cancels the old token between
            // setting Idle and swapping the token.
            let mut cancel_guard = self.cancel.lock().await;
            let mut state = self.state.lock().await;
            if *state != ServerState::Stopped {
                return Err(format!("voice server already running (state={:?})", *state));
            }

            let session = RecorderSession::open(app_config)
                .await
                .map_err(|e| format!("could not open the recording session: {e}"))?;
            match session.advance(app_config, SessionInput::Run).await {
                Ok(reply) if reply.accepted => *state = reply.phase.into(),
                other => {
                    let _ = session.close(app_config).await;
                    return Err(format!("recording session refused to start: {other:?}"));
                }
            }

            let fresh = CancellationToken::new();
            *cancel_guard = fresh.clone();
            let recorder = Recorder::new(session, self.state.clone());
            *self.recorder.lock().await = Some(recorder.clone());
            (fresh, recorder)
        };

        info!(
            "{LOG_PREFIX} starting voice server: hotkey={} mode={:?}",
            self.config.hotkey, self.config.activation_mode
        );

        // On macOS, the Fn/Globe key is intercepted by the system before
        // rdev's CGEventTap can see it. Use the Swift-based globe listener
        // instead, which monitors NSEvent.flagsChanged for the .function flag.
        let (listener_handle, mut hotkey_rx) = match start_hotkey_listener(
            ctx,
            app_config,
            &self.config.hotkey,
            self.config.activation_mode,
            &cancel,
        )
        .await
        {
            Ok(pair) => pair,
            Err(e) => {
                self.shut_session(app_config, &recorder).await;
                return Err(e);
            }
        };

        info!("{LOG_PREFIX} voice server ready, listening for hotkey");

        let mut recording: Option<RecordingHandle> = None;
        let mut recording_expected_app: Option<String> = None;

        // Pending recording setup: `start_recording()` runs on a blocking
        // thread so the event loop stays responsive to Release events that
        // macOS fires almost immediately for the Fn key.
        let mut recording_pending_rx: Option<RecordingSetupReceiver> = None;
        // Deferred stop deadline, set when the module answers `DeferStop`
        // because a stop intent arrived during setup. Keeping this in a
        // select! branch avoids blocking the hotkey loop.
        let mut deferred_stop_deadline: Option<tokio::time::Instant> = None;

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

                    let input = match event {
                        HotkeyEvent::Pressed => SessionInput::Pressed,
                        HotkeyEvent::Released => SessionInput::Released,
                    };
                    info!(
                        "{LOG_PREFIX} received hotkey event={event:?} recording={} pending={}",
                        recording.is_some(),
                        recording_pending_rx.is_some()
                    );
                    let Some(reply) = recorder.advance(app_config, input).await else {
                        continue;
                    };
                    match reply.action {
                        SessionAction::StartRecording { generation } => {
                            debug!("{LOG_PREFIX} hotkey pressed → starting recording generation={generation} (non-blocking)");
                            deferred_stop_deadline = None;

                            // Start recording on a blocking thread so the
                            // event loop remains responsive to Release.
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            let capture_config = app_config.clone();
                            ctx.spawn(async move {
                                let expected_app = capture_expected_app_name(&capture_config).await;
                                let result = audio_capture::start_recording(&capture_config).await
                                    .map(|handle| (handle, expected_app));
                                let _ = tx.send(result);
                            });
                            recording_pending_rx = Some(rx);
                        }
                        SessionAction::StopRecording { generation } => {
                            deferred_stop_deadline = None;
                            if let Some(handle) = recording.take() {
                                debug!("{LOG_PREFIX} hotkey stop → stopping recording generation={generation}");
                                self.spawn_process_recording(
                                    ctx,
                                    handle,
                                    app_config,
                                    &recorder,
                                    generation,
                                    recording_expected_app.take(),
                                );
                            }
                        }
                        SessionAction::None
                        | SessionAction::RecordingLive { .. }
                        | SessionAction::DeferStop { .. } => {}
                    }
                }

                result = pending_ready => {
                    // Recording setup completed (or failed).
                    recording_pending_rx = None;
                    match result {
                        Ok(Ok((handle, expected_app))) => {
                            // A hotkey event queued behind the setup result lost
                            // the select! race against `pending_ready` (on warm
                            // CPAL init both are ready at once). Either kind is
                            // a stop intent; the module decides what it means.
                            let stop_buffered = hotkey_rx.try_recv().is_ok();
                            if stop_buffered {
                                info!("{LOG_PREFIX} recording handle ready — found a buffered hotkey event (select! race recovered)");
                            }
                            match recorder
                                .advance(app_config, SessionInput::SetupReady { stop_buffered })
                                .await
                            {
                                Some(reply) if reply.accepted => {
                                    recording = Some(handle);
                                    recording_expected_app = expected_app;
                                    match reply.action {
                                        SessionAction::DeferStop { after_ms, .. } => {
                                            info!("{LOG_PREFIX} deferred stop: recording for at least {after_ms}ms");
                                            deferred_stop_deadline = Some(
                                                tokio::time::Instant::now()
                                                    + std::time::Duration::from_millis(after_ms),
                                            );
                                        }
                                        _ => {
                                            deferred_stop_deadline = None;
                                            info!("{LOG_PREFIX} recording started (live)");
                                        }
                                    }
                                }
                                _ => {
                                    // The session moved on (stopped, or the module
                                    // is unreachable): this recording has no owner.
                                    warn!("{LOG_PREFIX} recording setup result not accepted; dropping the recording");
                                    drop(handle);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            deferred_stop_deadline = None;
                            warn!("{LOG_PREFIX} failed to start recording: {e}");
                            recorder.advance(app_config, SessionInput::SetupFailed).await;
                            *self.last_error.lock().await = Some(e);
                        }
                        Err(_) => {
                            deferred_stop_deadline = None;
                            warn!("{LOG_PREFIX} recording setup task dropped");
                            recorder.advance(app_config, SessionInput::SetupFailed).await;
                        }
                    }
                }

                _ = deferred_stop_ready => {
                    deferred_stop_deadline = None;
                    if let Some(reply) = recorder
                        .advance(app_config, SessionInput::DeferredStopElapsed)
                        .await
                    {
                        if let SessionAction::StopRecording { generation } = reply.action {
                            if let Some(handle) = recording.take() {
                                info!("{LOG_PREFIX} deferred stop deadline reached, stopping recording");
                                self.spawn_process_recording(
                                    ctx,
                                    handle,
                                    app_config,
                                    &recorder,
                                    generation,
                                    recording_expected_app.take(),
                                );
                            }
                        }
                    }
                }

                _ = cancel.cancelled() => {
                    debug!("{LOG_PREFIX} cancellation received");
                    break;
                }
            }
        }

        listener_handle.stop().await;
        self.shut_session(app_config, &recorder).await;
        info!("{LOG_PREFIX} voice server stopped");

        Ok(())
    }

    /// Stop the module-side session (invalidating in-flight pipelines), close
    /// it, and leave the mirrored state `Stopped`.
    async fn shut_session(&self, config: &Config, recorder: &Recorder) {
        recorder.advance(config, SessionInput::Stop).await;
        recorder.close(config).await;
        *self.recorder.lock().await = None;
        // The module may have been unreachable; the server is stopped either way.
        *self.state.lock().await = ServerState::Stopped;
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
        ctx: &crate::core::runtime::DispatchCtx,
        handle: RecordingHandle,
        config: &Config,
        recorder: &Recorder,
        generation: u64,
        expected_app: Option<String>,
    ) {
        let pipeline_id = Uuid::new_v4().to_string()[..8].to_string();
        let server_config = self.config.clone();
        let transcription_count = self.transcription_count.clone();
        let recorder = recorder.clone();
        let last_error = self.last_error.clone();
        let recent_transcripts = self.recent_transcripts.clone();
        let app_config = config.clone();

        info!(
            "{LOG_PREFIX} [pipeline={pipeline_id}] spawning process_recording (generation={generation})"
        );

        ctx.spawn(async move {
            process_recording_bg(
                &pipeline_id,
                handle,
                &app_config,
                &server_config,
                recorder,
                transcription_count,
                generation,
                last_error,
                recent_transcripts,
                expected_app,
            )
            .await;
        });
    }
}
