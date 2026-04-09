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
    cancel: CancellationToken,
    config: VoiceServerConfig,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    last_error: Arc<Mutex<Option<String>>>,
    /// Rolling buffer of recent transcriptions used as whisper context for
    /// better continuity across consecutive recordings.
    recent_transcripts: Arc<Mutex<Vec<String>>>,
}

impl VoiceServer {
    pub fn new(config: VoiceServerConfig) -> Self {
        Self {
            state: Arc::new(Mutex::new(ServerState::Stopped)),
            cancel: CancellationToken::new(),
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
    pub async fn run(&self, app_config: &Config) -> Result<(), String> {
        info!(
            "{LOG_PREFIX} starting voice server: hotkey={} mode={:?}",
            self.config.hotkey, self.config.activation_mode
        );

        let combo = hotkey::parse_hotkey(&self.config.hotkey)?;
        let (listener_handle, mut hotkey_rx) =
            hotkey::start_listener(combo, self.config.activation_mode)?;

        *self.state.lock().await = ServerState::Idle;

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
                                        recording_expected_app.take(),
                                    );
                                }
                            } else if recording_pending_rx.is_some() {
                                info!("{LOG_PREFIX} hotkey pressed while recording setup pending — buffering stop intent");
                                pending_stop = true;
                            } else {
                                let expected_app = capture_expected_app_name();
                                debug!("{LOG_PREFIX} hotkey pressed → starting recording (non-blocking)");

                                // Start recording on a blocking thread so the
                                // event loop remains responsive to Release.
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                tokio::task::spawn_blocking(move || {
                                    let result = audio_capture::start_recording();
                                    let _ = tx.send(result);
                                });
                                recording_pending_rx = Some(rx);
                                pending_expected_app = expected_app;
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
                            info!("{LOG_PREFIX} recording handle ready (pending_stop={pending_stop})");
                            if pending_stop {
                                // A stop intent arrived while cpal was initialising.
                                // Keep recording for a minimum duration, then stop
                                // via non-blocking deferred deadline branch.
                                pending_stop = false;
                                recording = Some(handle);
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
                                recording_expected_app = pending_expected_app.take();
                                deferred_stop_deadline = None;

                                info!("{LOG_PREFIX} recording started (live)");
                            }
                        }
                        Ok(Err(e)) => {
                            pending_stop = false;
                            deferred_stop_deadline = None;
                            pending_expected_app = None;
                            error!("{LOG_PREFIX} failed to start recording: {e}");
                            *self.state.lock().await = ServerState::Idle;
                            *self.last_error.lock().await = Some(e);
                        }
                        Err(_) => {
                            pending_stop = false;
                            deferred_stop_deadline = None;
                            pending_expected_app = None;
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
                            recording_expected_app.take(),
                        );
                    }
                }

                _ = self.cancel.cancelled() => {
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

    /// Stop the voice server.
    pub async fn stop(&self) {
        info!("{LOG_PREFIX} stopping voice server");
        self.cancel.cancel();
    }

    /// Spawn `process_recording` as a background task so the hotkey event
    /// loop is not blocked during transcription. This ensures rapid
    /// consecutive Fn presses are never missed.
    fn spawn_process_recording(
        &self,
        handle: RecordingHandle,
        config: &Config,
        expected_app: Option<String>,
    ) {
        let state = self.state.clone();
        let server_config = self.config.clone();
        let transcription_count = self.transcription_count.clone();
        let last_error = self.last_error.clone();
        let recent_transcripts = self.recent_transcripts.clone();
        let app_config = config.clone();

        tokio::spawn(async move {
            process_recording_bg(
                handle,
                &app_config,
                &server_config,
                state,
                transcription_count,
                last_error,
                recent_transcripts,
                expected_app,
            )
            .await;
        });
    }
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
    if prompt.len() > MAX_INITIAL_PROMPT_CHARS {
        prompt.truncate(MAX_INITIAL_PROMPT_CHARS);
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
    handle: RecordingHandle,
    config: &Config,
    server_config: &VoiceServerConfig,
    state: Arc<Mutex<ServerState>>,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    last_error: Arc<Mutex<Option<String>>>,
    recent_transcripts: Arc<Mutex<Vec<String>>>,
    expected_app: Option<String>,
) {
    let pipeline_started = Instant::now();
    *state.lock().await = ServerState::Transcribing;

    let stop_started = Instant::now();
    match handle.stop().await {
        Ok(result) => {
            let stop_elapsed = stop_started.elapsed();
            info!(
                "{LOG_PREFIX} recording stopped: {:.1}s, {} bytes, peak_rms={:.6} (stop_elapsed_ms={})",
                result.duration_secs,
                result.wav_bytes.len(),
                result.peak_rms,
                stop_elapsed.as_millis()
            );

            // Gate 1: minimum duration.
            if result.duration_secs < server_config.min_duration_secs {
                warn!(
                    "{LOG_PREFIX} recording too short ({:.1}s), skipping",
                    result.duration_secs
                );
                *state.lock().await = ServerState::Idle;
                return;
            }

            // Gate 2: silence detection.
            if result.peak_rms < server_config.silence_threshold {
                warn!(
                    "{LOG_PREFIX} audio is silence (peak_rms={:.6} < threshold={:.6}), skipping transcription",
                    result.peak_rms,
                    server_config.silence_threshold
                );
                *state.lock().await = ServerState::Idle;
                return;
            }

            // Build initial_prompt from dictionary + recent transcripts.
            let initial_prompt = build_initial_prompt(server_config, &recent_transcripts).await;
            let context = initial_prompt
                .as_deref()
                .or(server_config.context.as_deref());
            if let Some(app) = expected_app.as_deref() {
                debug!("{LOG_PREFIX} insertion target captured on hotkey press: app='{app}'");
            } else {
                debug!("{LOG_PREFIX} insertion target unknown at hotkey press");
            }

            info!(
                "{LOG_PREFIX} transcribing: skip_cleanup={} context={}",
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
                        "{LOG_PREFIX} transcription: '{}' ({} chars, transcribe_elapsed_ms={})",
                        truncate_for_log(text, 80),
                        text.len(),
                        transcribe_elapsed.as_millis()
                    );

                    // Gate 3: filter hallucinated/blank output.
                    if is_hallucinated_output(text) {
                        warn!(
                            "{LOG_PREFIX} detected hallucinated output, discarding: '{}'",
                            truncate_for_log(text, 60)
                        );
                        *state.lock().await = ServerState::Idle;
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
                            super::dictation_listener::publish_transcription(text.to_string());
                            transcription_count.fetch_add(1, Ordering::Relaxed);
                            info!(
                                "{LOG_PREFIX} app is focused — delivered via Socket.IO (total_pipeline_ms={})",
                                pipeline_started.elapsed().as_millis()
                            );
                        } else {
                            let insert_started = Instant::now();
                            if let Err(e) = text_input::insert_text(text, expected_app.as_deref()) {
                                error!("{LOG_PREFIX} failed to insert text: {e}");
                                *last_error.lock().await = Some(e);
                            } else {
                                let insert_elapsed = insert_started.elapsed();
                                transcription_count.fetch_add(1, Ordering::Relaxed);
                                info!(
                                    "{LOG_PREFIX} text inserted into active field (insert_elapsed_ms={}, total_pipeline_ms={})",
                                    insert_elapsed.as_millis(),
                                    pipeline_started.elapsed().as_millis()
                                );
                            }
                        }
                    } else {
                        debug!("{LOG_PREFIX} transcription was empty, nothing to insert");
                    }
                }
                Err(e) => {
                    error!("{LOG_PREFIX} transcription failed: {e}");
                    *last_error.lock().await = Some(e);
                }
            }
        }
        Err(e) => {
            error!("{LOG_PREFIX} failed to stop recording: {e}");
            *last_error.lock().await = Some(e);
        }
    }

    debug!(
        "{LOG_PREFIX} process_recording finished (total_pipeline_ms={})",
        pipeline_started.elapsed().as_millis()
    );
    *state.lock().await = ServerState::Idle;
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
    tokio::spawn(async move {
        if let Err(e) = server.run(&config_for_run).await {
            error!("{LOG_PREFIX} embedded voice server exited with error: {e}");
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

/// Known whisper hallucination patterns. These are common outputs when
/// whisper processes near-silent audio or audio with background noise.
/// Sourced from community lists and OpenWhispr's filtering behavior.
const HALLUCINATION_PATTERNS: &[&str] = &[
    // whisper.cpp blank markers
    "[blank_audio]",
    "[ blank_audio ]",
    "[blank audio]",
    "(blank audio)",
    // Common hallucinations from YouTube-trained models
    "thank you",
    "thank you.",
    "thanks.",
    "thank you for watching",
    "thanks for watching",
    "thank you for listening",
    "thanks for listening",
    "thank you so much",
    "please subscribe",
    "like and subscribe",
    "see you next time",
    "see you in the next video",
    "bye bye",
    "bye.",
    "goodbye.",
    // Single-word noise artifacts
    "you",
    "the",
    "i",
    "a",
    "so",
    "okay",
    "ok",
    "yeah",
    "yes",
    "no",
    "oh",
    "hmm",
    "huh",
    "ah",
    // Punctuation-only
    "...",
    ".",
    ",",
    "!",
    "?",
];

/// Check if whisper output is a known hallucination pattern.
///
/// Whisper.cpp famously outputs "[BLANK_AUDIO]" for silence and various
/// stock phrases ("Thank you for watching", etc.) when fed noisy or
/// near-empty audio. Filtering these prevents inserting garbage text.
fn is_hallucinated_output(text: &str) -> bool {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return false; // handled separately as "empty"
    }

    // Strip trailing punctuation for matching (whisper often appends periods).
    let stripped = normalized.trim_end_matches(|c: char| c.is_ascii_punctuation());

    // Exact match against known hallucination phrases.
    for pattern in HALLUCINATION_PATTERNS {
        if normalized == *pattern || stripped == *pattern {
            return true;
        }
    }

    // Detect repeated short phrases (e.g. "you you you you").
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.len() >= 3 {
        let first = words[0];
        if words.iter().all(|w| *w == first) {
            return true;
        }
    }

    false
}

fn truncate_for_log(s: &str, max: usize) -> String {
    let truncated: String = s.chars().take(max).collect();
    if truncated.len() < s.len() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_server_config() {
        let cfg = VoiceServerConfig::default();
        assert_eq!(cfg.hotkey, "Fn");
        assert_eq!(cfg.activation_mode, ActivationMode::Push);
        assert!(!cfg.skip_cleanup);
        assert!(cfg.context.is_none());
        assert!(cfg.custom_dictionary.is_empty());
        assert!((cfg.silence_threshold - DEFAULT_SILENCE_THRESHOLD).abs() < 1e-6);
    }

    #[test]
    fn hallucination_detection() {
        // Blank audio markers.
        assert!(is_hallucinated_output("[BLANK_AUDIO]"));
        assert!(is_hallucinated_output("  [blank_audio]  "));
        assert!(is_hallucinated_output("[ BLANK_AUDIO ]"));
        // Common hallucinated phrases.
        assert!(is_hallucinated_output("Thank you for watching"));
        assert!(is_hallucinated_output("thanks for listening"));
        assert!(is_hallucinated_output("Thank you."));
        assert!(is_hallucinated_output("Thank you"));
        assert!(is_hallucinated_output("Thanks."));
        assert!(is_hallucinated_output("Bye."));
        assert!(is_hallucinated_output("Goodbye."));
        // Repeated words.
        assert!(is_hallucinated_output("you you you you"));
        assert!(is_hallucinated_output("the the the the"));
        // Punctuation-only.
        assert!(is_hallucinated_output("..."));
        assert!(is_hallucinated_output("."));
        // Single noise words.
        assert!(is_hallucinated_output("you"));
        assert!(is_hallucinated_output("Yeah"));
        assert!(is_hallucinated_output("Hmm"));
        assert!(is_hallucinated_output("Oh."));
        // Should NOT flag real speech.
        assert!(!is_hallucinated_output("Hello, how are you?"));
        assert!(!is_hallucinated_output("the quick brown fox"));
        assert!(!is_hallucinated_output("I want to order pizza"));
        assert!(!is_hallucinated_output(
            "thank you for your help with the project"
        ));
        assert!(!is_hallucinated_output(""));
    }

    #[tokio::test]
    async fn server_status_initial() {
        let server = VoiceServer::new(VoiceServerConfig::default());
        let status = server.status().await;
        assert_eq!(status.state, ServerState::Stopped);
        assert_eq!(status.transcription_count, 0);
        assert!(status.last_error.is_none());
    }

    #[test]
    fn server_state_serializes() {
        let json = serde_json::to_string(&ServerState::Recording).unwrap();
        assert_eq!(json, "\"recording\"");
    }

    #[test]
    fn voice_server_status_serializes() {
        let status = VoiceServerStatus {
            state: ServerState::Idle,
            hotkey: "Fn".into(),
            activation_mode: ActivationMode::Push,
            transcription_count: 5,
            last_error: None,
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["state"], "idle");
        assert_eq!(v["transcription_count"], 5);
    }

    #[test]
    fn truncate_for_log_short() {
        assert_eq!(truncate_for_log("hello", 10), "hello");
    }

    #[test]
    fn truncate_for_log_long() {
        let result = truncate_for_log("hello world this is long", 10);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 14); // 10 + "..."
    }
}
