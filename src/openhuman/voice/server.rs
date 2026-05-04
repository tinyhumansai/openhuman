//! Standalone voice server — hotkey → record → transcribe → insert text.
//!
//! Can run as part of the core process or independently via the CLI.
//! The server listens for a configurable hotkey, records audio from the
//! microphone, transcribes via whisper, and inserts the result into the
//! active text field.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[cfg(target_os = "macos")]
use crate::openhuman::accessibility;
use crate::openhuman::config::Config;

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

/// Maximum number of recent transcriptions to keep as context for whisper's
/// initial_prompt, improving continuity across consecutive recordings.
const MAX_RECENT_TRANSCRIPTS: usize = 5;

/// Maximum character length of the combined initial prompt (dictionary +
/// recent transcripts). Whisper's prompt token budget is limited.
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
    /// Custom vocabulary words to bias whisper toward (passed as initial_prompt).
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
    /// Rolling buffer of recent transcriptions used as whisper context for
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
                            error!("{LOG_PREFIX} failed to start recording: {e}");
                            *self.state.lock().await = ServerState::Idle;
                            *self.last_error.lock().await = Some(e);
                        }
                        Err(_) => {
                            pending_stop = false;
                            deferred_stop_deadline = None;
                            pending_expected_app = None;
                            pending_generation = None;
                            error!("{LOG_PREFIX} recording setup task dropped");
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
/// On macOS, the Fn/Globe key cannot be detected by `rdev`'s CGEventTap.
/// When the configured hotkey is `"fn"` we fall back to the Swift-based
/// globe listener (`accessibility::globe`) which monitors
/// `NSEvent.flagsChanged` for the `.function` modifier flag.
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
    }

    // Default path: rdev-based listener for all other keys.
    let combo = hotkey::parse_hotkey(hotkey_str)?;
    let (handle, rx) = hotkey::start_listener(combo, mode)?;
    Ok((HotkeyListenerKind::Rdev(handle), rx))
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
    use crate::openhuman::accessibility::{globe_listener_poll, globe_listener_start};

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

// ── Background processing (free functions, spawnable) ─────────────────

/// Capture the frontmost app name at hotkey press so insertion can be validated later.
#[cfg(target_os = "macos")]
fn capture_expected_app_name() -> Option<String> {
    match accessibility::focused_text_context_verbose() {
        Ok(ctx) => {
            let app = ctx
                .app_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let Some(app_name) = app {
                debug!("{LOG_PREFIX} captured focused app on press: '{app_name}'");
                Some(app_name.to_string())
            } else {
                debug!("{LOG_PREFIX} focus query returned no app name on press");
                None
            }
        }
        Err(e) => {
            warn!("{LOG_PREFIX} failed to capture focused app on press: {e}");
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn capture_expected_app_name() -> Option<String> {
    None
}

/// Build the whisper initial_prompt from custom dictionary + recent transcripts.
async fn build_initial_prompt(
    config: &VoiceServerConfig,
    recent_transcripts: &Mutex<Vec<String>>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if !config.custom_dictionary.is_empty() {
        parts.push(config.custom_dictionary.join(", "));
    }

    let recent = recent_transcripts.lock().await;
    if !recent.is_empty() {
        parts.push(recent.join(" "));
    }

    if parts.is_empty() {
        return None;
    }

    let mut prompt = parts.join(". ");
    if prompt.chars().count() > MAX_INITIAL_PROMPT_CHARS {
        prompt = prompt.chars().take(MAX_INITIAL_PROMPT_CHARS).collect();
        if let Some(last_space) = prompt.rfind(' ') {
            prompt.truncate(last_space);
        }
    }
    debug!(
        "{LOG_PREFIX} built initial_prompt ({} chars): '{}'",
        prompt.len(),
        truncate_for_log(&prompt, 100)
    );
    Some(prompt)
}

/// Add a transcript to the rolling recent buffer.
async fn push_recent_transcript(recent_transcripts: &Mutex<Vec<String>>, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let mut recent = recent_transcripts.lock().await;
    recent.push(trimmed.to_string());
    while recent.len() > MAX_RECENT_TRANSCRIPTS {
        recent.remove(0);
    }
}

/// Process a completed recording in the background.
///
/// This is a free function (not `&self`) so it can be spawned via
/// `tokio::spawn` without blocking the hotkey event loop. All shared
/// state is passed as `Arc` handles.
#[allow(clippy::too_many_arguments)]
async fn process_recording_bg(
    pipeline_id: &str,
    handle: RecordingHandle,
    config: &Config,
    server_config: &VoiceServerConfig,
    state: Arc<Mutex<ServerState>>,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    session_generation: Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
    last_error: Arc<Mutex<Option<String>>>,
    recent_transcripts: Arc<Mutex<Vec<String>>>,
    expected_app: Option<String>,
) {
    let pipeline_started = Instant::now();
    info!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=start generation={generation}");
    update_state_if_current(
        &state,
        &session_generation,
        generation,
        ServerState::Transcribing,
        "transcribing",
    )
    .await;

    let stop_started = Instant::now();
    match handle.stop().await {
        Ok(result) => {
            let stop_elapsed = stop_started.elapsed();
            info!(
                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=stop_recording duration={:.1}s bytes={} peak_rms={:.6} stop_elapsed_ms={}",
                result.duration_secs,
                result.wav_bytes.len(),
                result.peak_rms,
                stop_elapsed.as_millis()
            );

            // Gate 1: minimum duration.
            if result.duration_secs < server_config.min_duration_secs {
                warn!(
                    "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_duration DROPPED ({:.1}s < {:.1}s min)",
                    result.duration_secs,
                    server_config.min_duration_secs
                );
                update_state_if_current(
                    &state,
                    &session_generation,
                    generation,
                    ServerState::Idle,
                    "idle_after_short_recording",
                )
                .await;
                return;
            }

            // Gate 2: silence detection.
            if result.peak_rms < server_config.silence_threshold {
                warn!(
                    "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_silence DROPPED (peak_rms={:.6} < threshold={:.6})",
                    result.peak_rms,
                    server_config.silence_threshold
                );
                update_state_if_current(
                    &state,
                    &session_generation,
                    generation,
                    ServerState::Idle,
                    "idle_after_silence",
                )
                .await;
                return;
            }

            // Build initial_prompt from dictionary + recent transcripts.
            let initial_prompt = build_initial_prompt(server_config, &recent_transcripts).await;
            let context = initial_prompt
                .as_deref()
                .or(server_config.context.as_deref());
            if let Some(app) = expected_app.as_deref() {
                debug!("{LOG_PREFIX} [pipeline={pipeline_id}] insertion target: app='{app}'");
            } else {
                debug!("{LOG_PREFIX} [pipeline={pipeline_id}] insertion target unknown");
            }

            info!(
                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=transcribe skip_cleanup={} context={}",
                server_config.skip_cleanup,
                context.map_or("none".to_string(), |c| format!("{}chars", c.len()))
            );

            let transcribe_started = Instant::now();
            match crate::openhuman::voice::voice_transcribe_bytes(
                config,
                &result.wav_bytes,
                Some("wav".to_string()),
                context,
                server_config.skip_cleanup,
            )
            .await
            {
                Ok(outcome) => {
                    let transcribe_elapsed = transcribe_started.elapsed();
                    let text = &outcome.value.text;
                    info!(
                        "{LOG_PREFIX} [pipeline={pipeline_id}] stage=transcription_result text='{}' chars={} elapsed_ms={}",
                        truncate_for_log(text, 80),
                        text.len(),
                        transcribe_elapsed.as_millis()
                    );

                    // Gate 3: filter hallucinated/blank output.
                    if is_hallucinated_output(text, HallucinationMode::Dictation) {
                        warn!(
                            "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_hallucination DROPPED text='{}'",
                            truncate_for_log(text, 60)
                        );
                        update_state_if_current(
                            &state,
                            &session_generation,
                            generation,
                            ServerState::Idle,
                            "idle_after_hallucination",
                        )
                        .await;
                        return;
                    }

                    if !text.trim().is_empty() {
                        push_recent_transcript(&recent_transcripts, text).await;

                        // When the Tauri app itself is focused, deliver via
                        // Socket.IO so the frontend inserts into the chat.
                        // Otherwise paste via OS-level Cmd+V into the
                        // external app.
                        let is_self = expected_app
                            .as_deref()
                            .map(|app| app.to_lowercase().contains("openhuman"))
                            .unwrap_or(false);

                        if is_self {
                            let receivers =
                                super::dictation_listener::publish_transcription(text.to_string());
                            transcription_count.fetch_add(1, Ordering::Relaxed);
                            info!(
                                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_socketio receivers={receivers} total_pipeline_ms={}",
                                pipeline_started.elapsed().as_millis()
                            );
                        } else {
                            let insert_started = Instant::now();
                            if let Err(e) = text_input::insert_text(text, expected_app.as_deref()) {
                                error!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_paste FAILED: {e}");
                                *last_error.lock().await = Some(e);
                            } else {
                                let insert_elapsed = insert_started.elapsed();
                                transcription_count.fetch_add(1, Ordering::Relaxed);
                                info!(
                                    "{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_paste insert_ms={} total_pipeline_ms={}",
                                    insert_elapsed.as_millis(),
                                    pipeline_started.elapsed().as_millis()
                                );
                            }
                        }
                    } else {
                        warn!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_empty DROPPED (transcription was blank)");
                    }
                }
                Err(e) => {
                    error!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=transcribe FAILED: {e}");
                    *last_error.lock().await = Some(e);
                }
            }
        }
        Err(e) => {
            error!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=stop_recording FAILED: {e}");
            *last_error.lock().await = Some(e);
        }
    }

    info!(
        "{LOG_PREFIX} [pipeline={pipeline_id}] stage=done total_pipeline_ms={}",
        pipeline_started.elapsed().as_millis()
    );
    update_state_if_current(
        &state,
        &session_generation,
        generation,
        ServerState::Idle,
        "idle_after_processing",
    )
    .await;
}

async fn update_state_if_current(
    state: &Arc<Mutex<ServerState>>,
    session_generation: &Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
    next_state: ServerState,
    reason: &str,
) {
    let latest_generation = session_generation.load(Ordering::Relaxed);
    if latest_generation != generation {
        debug!(
            "{LOG_PREFIX} skipped stale state update reason={} generation={} latest_generation={} next_state={next_state:?}",
            reason,
            generation,
            latest_generation
        );
        return;
    }

    debug!(
        "{LOG_PREFIX} state update reason={} generation={} next_state={next_state:?}",
        reason, generation
    );
    *state.lock().await = next_state;
}

/// Global voice server instance, lazily initialized.
static VOICE_SERVER: once_cell::sync::OnceCell<Arc<VoiceServer>> = once_cell::sync::OnceCell::new();

/// Get or initialize the global voice server instance.
pub fn global_server(config: VoiceServerConfig) -> Arc<VoiceServer> {
    VOICE_SERVER
        .get_or_init(|| Arc::new(VoiceServer::new(config)))
        .clone()
}

/// Get the global voice server if already initialized.
pub fn try_global_server() -> Option<Arc<VoiceServer>> {
    VOICE_SERVER.get().cloned()
}

/// Start the embedded global voice server when config enables auto-start.
///
/// This is intended for core process startup. The server runs in the background
/// and reuses the process-global singleton so RPC status/stop calls continue to
/// operate on the same instance.
pub async fn start_if_enabled(app_config: &Config) {
    if !app_config.voice_server.auto_start {
        info!("{LOG_PREFIX} auto-start disabled in config, skipping embedded voice server");
        return;
    }

    let server_config = VoiceServerConfig {
        hotkey: app_config.voice_server.hotkey.clone(),
        activation_mode: match app_config.voice_server.activation_mode {
            crate::openhuman::config::VoiceActivationMode::Tap => ActivationMode::Tap,
            crate::openhuman::config::VoiceActivationMode::Push => ActivationMode::Push,
        },
        skip_cleanup: app_config.voice_server.skip_cleanup,
        context: None,
        min_duration_secs: app_config.voice_server.min_duration_secs,
        silence_threshold: app_config.voice_server.silence_threshold,
        custom_dictionary: app_config.voice_server.custom_dictionary.clone(),
    };

    if let Some(existing) = try_global_server() {
        let status = existing.status().await;
        if status.state != ServerState::Stopped {
            info!(
                "{LOG_PREFIX} embedded voice server already running: hotkey={} mode={:?}",
                status.hotkey, status.activation_mode
            );
            return;
        }
    }

    info!(
        "{LOG_PREFIX} auto-start enabled, launching embedded voice server: hotkey={} mode={:?}",
        server_config.hotkey, server_config.activation_mode
    );

    let server = global_server(server_config);
    let config_for_run = app_config.clone();
    let server_for_err = server.clone();
    tokio::spawn(async move {
        if let Err(e) = server.run(&config_for_run).await {
            error!("{LOG_PREFIX} embedded voice server exited with error: {e}");
            server_for_err.set_last_error(&e).await;
        }
    });
}

/// Run the voice server standalone (blocking). Intended for CLI usage.
///
/// Creates a fresh `VoiceServer` that is **not** registered in the global
/// singleton used by `voice_server_status` RPC. This keeps CLI-started
/// instances isolated from the core RPC lifecycle.
pub async fn run_standalone(
    app_config: Config,
    server_config: VoiceServerConfig,
) -> Result<(), String> {
    info!("{LOG_PREFIX} starting standalone voice server");
    info!("{LOG_PREFIX} hotkey: {}", server_config.hotkey);
    info!("{LOG_PREFIX} mode: {:?}", server_config.activation_mode);
    info!("{LOG_PREFIX} press the hotkey to start dictating");

    let server = VoiceServer::new(server_config);

    // Handle Ctrl+C gracefully.
    let server_arc = Arc::new(server);
    let server_for_signal = server_arc.clone();

    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            info!("{LOG_PREFIX} Ctrl+C received, shutting down");
            server_for_signal.stop().await;
        }
    });

    // This is safe because we hold the Arc and nothing else moves it.
    // The server.run() borrows &self, and we await it to completion.
    server_arc.run(&app_config).await
}

// Hallucination detection is now in the shared `hallucination` module.
use super::hallucination::{is_hallucinated_output, HallucinationMode};

fn truncate_for_log(s: &str, max: usize) -> String {
    let truncated: String = s.chars().take(max).collect();
    if truncated.len() < s.len() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
