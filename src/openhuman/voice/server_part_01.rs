use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{debug, info, warn};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::openhuman::config::Config;
#[cfg(target_os = "macos")]
use crate::openhuman::desktop::accessibility;

use super::audio_capture::{self, RecordingHandle};
use super::hotkey::{self, ActivationMode, HotkeyEvent};
use super::text_input;

const LOG_PREFIX: &str = "[voice_server]";

/// Running state of the voice server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    /// Server is not running.
    Stopped,
    /// Server is running and idle, waiting for hotkey.
    Idle,
    /// Actively recording audio.
    Recording,
    /// Transcribing recorded audio.
    Transcribing,
}

/// Status snapshot of the voice server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VoiceServerStatus {
    pub state: ServerState,
    pub hotkey: String,
    pub activation_mode: ActivationMode,
    pub transcription_count: u64,
    pub last_error: Option<String>,
}

/// Default silence threshold (RMS energy). Recordings with peak RMS below
/// this are considered silent and skipped. Matches OpenWhispr's 0.002 default.
const DEFAULT_SILENCE_THRESHOLD: f32 = 0.002;

/// Maximum number of recent transcriptions to keep as context for the STT engine's
/// initial_prompt, improving continuity across consecutive recordings.
const MAX_RECENT_TRANSCRIPTS: usize = 5;

/// Maximum character length of the combined initial prompt (dictionary +
/// recent transcripts). The prompt token budget is limited.
const MAX_INITIAL_PROMPT_CHARS: usize = 500;

/// Configuration for the voice server.
#[derive(Debug, Clone)]
pub struct VoiceServerConfig {
    pub hotkey: String,
    pub activation_mode: ActivationMode,
    /// Skip LLM post-processing on transcriptions.
    pub skip_cleanup: bool,
    /// Optional conversation context for better transcription accuracy.
    pub context: Option<String>,
    /// Minimum recording duration in seconds. Shorter recordings are discarded.
    pub min_duration_secs: f32,
    /// RMS energy threshold for silence detection. Recordings with peak
    /// energy below this are treated as silence and skipped.
    pub silence_threshold: f32,
    /// Custom vocabulary words to bias the STT engine toward (passed as initial_prompt).
    pub custom_dictionary: Vec<String>,
}

impl Default for VoiceServerConfig {
    fn default() -> Self {
        Self {
            hotkey: "Fn".to_string(),
            activation_mode: ActivationMode::Push,
            skip_cleanup: false,
            context: None,
            min_duration_secs: 0.3,
            silence_threshold: DEFAULT_SILENCE_THRESHOLD,
            custom_dictionary: Vec::new(),
        }
    }
}

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
            &self.config.hotkey,
            self.config.activation_mode,
            &cancel,
        ) {
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
            tokio::sync::oneshot::Receiver<Result<RecordingHandle, String>>,
        > = None;
        let mut pending_expected_app: Option<String> = None;
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
                    // dictation_listener is not running (single rdev listener).
                    {
                        use super::dictation_listener;
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
                                let expected_app = capture_expected_app_name();
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
                                tokio::task::spawn_blocking(move || {
                                    let result = audio_capture::start_recording();
                                    let _ = tx.send(result);
                                });
                                recording_pending_rx = Some(rx);
                                pending_expected_app = expected_app;
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
                        Ok(Ok(handle)) => {
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
                                recording_expected_app = pending_expected_app.take();

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
                                recording_expected_app = pending_expected_app.take();
                                deferred_stop_deadline = None;

                                info!("{LOG_PREFIX} recording started (live)");
                            }
                        }
                        Ok(Err(e)) => {
                            pending_stop = false;
                            deferred_stop_deadline = None;
                            pending_expected_app = None;
                            pending_generation = None;
                            warn!("{LOG_PREFIX} failed to start recording: {e}");
                            *self.state.lock().await = ServerState::Idle;
                            *self.last_error.lock().await = Some(e);
                        }
                        Err(_) => {
                            pending_stop = false;
                            deferred_stop_deadline = None;
                            pending_expected_app = None;
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

        listener_handle.stop();
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

        tokio::spawn(async move {
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

// ── Hotkey listener dispatch (rdev vs macOS globe helper) ─────────────

/// Opaque handle that keeps the hotkey listener alive. Drop to stop.
enum HotkeyListenerKind {
    Rdev(hotkey::HotkeyListenerHandle),
    #[cfg(target_os = "macos")]
    Globe(CancellationToken),
}

impl HotkeyListenerKind {
    fn stop(&self) {
        match self {
            HotkeyListenerKind::Rdev(handle) => handle.stop(),
            #[cfg(target_os = "macos")]
            HotkeyListenerKind::Globe(cancel) => cancel.cancel(),
        }
    }
}

/// Start the appropriate hotkey listener for the current platform and key.
///
/// On macOS, the Fn/Globe key is handled by the Swift-based globe listener
/// (`accessibility::globe`) which monitors `NSEvent.flagsChanged`. All other
/// keys return an error on macOS: rdev's CGEventTap callback calls
/// `TSMGetInputSourceProperty` off the main thread; macOS 26 enforces
/// `dispatch_assert_queue(main_queue)` inside that API and kills the process
/// with `EXC_BREAKPOINT` (`dispatch_assert_queue_fail`). Configure
/// `hotkey = "fn"` to avoid this. (#2677)
fn start_hotkey_listener(
    hotkey_str: &str,
    mode: hotkey::ActivationMode,
    server_cancel: &CancellationToken,
) -> Result<
    (
        HotkeyListenerKind,
        tokio::sync::mpsc::UnboundedReceiver<hotkey::HotkeyEvent>,
    ),
    String,
> {
    #[cfg(target_os = "macos")]
    {
        if hotkey_str.trim().eq_ignore_ascii_case("fn") {
            return start_globe_hotkey_listener(mode, server_cancel);
        }
        // rdev calls TSMGetInputSourceProperty off the main thread; macOS 26
        // enforces main-queue-only access and crashes the process. Only the
        // Fn/Globe key is safe via the Swift globe listener. (#2677)
        Err(format!(
            "voice server hotkey '{}' is not supported on macOS — \
             only 'fn' (Fn/Globe key) is safe. rdev calls \
             TSMGetInputSourceProperty off the main thread on macOS 26, \
             causing EXC_BREAKPOINT. Set hotkey = \"fn\" in voice config \
             (issue #2677).",
            hotkey_str
        ))
    }

    // Non-macOS: rdev-based listener for all keys.
    #[cfg(not(target_os = "macos"))]
    {
        // `server_cancel` is only consumed by the macOS Swift-globe listener
        // branch above; the rdev listener manages its own lifecycle, so bind it
        // here to keep the shared signature warning-free on non-macOS.
        let _ = server_cancel;
        let combo = hotkey::parse_hotkey(hotkey_str)?;
        let (handle, rx) = hotkey::start_listener(combo, mode)?;
        Ok((HotkeyListenerKind::Rdev(handle), rx))
    }
}

/// macOS-only: start the Swift globe listener and bridge FN_DOWN / FN_UP
/// events into `HotkeyEvent::Pressed` / `HotkeyEvent::Released`.
#[cfg(target_os = "macos")]
fn start_globe_hotkey_listener(
    mode: hotkey::ActivationMode,
    server_cancel: &CancellationToken,
) -> Result<
    (
        HotkeyListenerKind,
        tokio::sync::mpsc::UnboundedReceiver<hotkey::HotkeyEvent>,
    ),
    String,
> {
    use crate::openhuman::desktop::accessibility::{globe_listener_poll, globe_listener_start};

    info!("{LOG_PREFIX} hotkey is Fn on macOS — using Swift globe listener instead of rdev");

    let status = globe_listener_start()?;
    if !status.running {
        let err_msg = status
            .last_error
            .unwrap_or_else(|| "globe listener failed to start".to_string());
        return Err(format!("globe listener: {err_msg}"));
    }
    info!(
        "{LOG_PREFIX} globe listener started, permission={:?}",
        status.input_monitoring_permission
    );

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = server_cancel.child_token();
    let cancel_clone = cancel.clone();

    // Tap mode state: track whether we're currently active.
    let is_active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    tokio::spawn(async move {
        let mut poll_interval = tokio::time::interval(Duration::from_millis(50));
        poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    debug!("{LOG_PREFIX} globe poller cancelled");
                    break;
                }
                _ = poll_interval.tick() => {
                    let poll_result = match globe_listener_poll() {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("{LOG_PREFIX} globe poll error: {e}");
                            continue;
                        }
                    };

                    for event_str in &poll_result.events {
                        let hotkey_event = match event_str.as_str() {
                            "FN_DOWN" => match mode {
                                hotkey::ActivationMode::Push => {
                                    Some(hotkey::HotkeyEvent::Pressed)
                                }
                                hotkey::ActivationMode::Tap => {
                                    let was_active = is_active.load(std::sync::atomic::Ordering::SeqCst);
                                    if was_active {
                                        is_active.store(false, std::sync::atomic::Ordering::SeqCst);
                                        Some(hotkey::HotkeyEvent::Released)
                                    } else {
                                        is_active.store(true, std::sync::atomic::Ordering::SeqCst);
                                        Some(hotkey::HotkeyEvent::Pressed)
                                    }
                                }
                            },
                            "FN_UP" => match mode {
                                hotkey::ActivationMode::Push => {
                                    Some(hotkey::HotkeyEvent::Released)
                                }
                                hotkey::ActivationMode::Tap => None, // tap ignores release
                            },
                            _ => None, // ignore modifier events
                        };

                        if let Some(ev) = hotkey_event {
                            debug!("{LOG_PREFIX} globe event {event_str} → {ev:?}");
                            if tx.send(ev).is_err() {
                                debug!("{LOG_PREFIX} globe poller: receiver dropped, stopping");
                                return;
                            }
                        }
                    }
                }
            }
        }
    });

    Ok((HotkeyListenerKind::Globe(cancel), rx))
}
