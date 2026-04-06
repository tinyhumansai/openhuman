//! Standalone voice server — hotkey → record → transcribe → insert text.
//!
//! Can run as part of the core process or independently via the CLI.
//! The server listens for a configurable hotkey, records audio from the
//! microphone, transcribes via whisper, and inserts the result into the
//! active text field.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::{debug, error, info, warn};
use tokio::sync::Mutex;

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

/// Configuration for the voice server.
#[derive(Debug, Clone)]
pub struct VoiceServerConfig {
    pub hotkey: String,
    pub activation_mode: ActivationMode,
    /// Skip LLM post-processing on transcriptions.
    pub skip_cleanup: bool,
    /// Optional conversation context for better transcription accuracy.
    pub context: Option<String>,
}

impl Default for VoiceServerConfig {
    fn default() -> Self {
        Self {
            hotkey: "ctrl+shift+space".to_string(),
            activation_mode: ActivationMode::Tap,
            skip_cleanup: false,
            context: None,
        }
    }
}

/// The voice server runtime.
pub struct VoiceServer {
    state: Arc<Mutex<ServerState>>,
    running: Arc<AtomicBool>,
    config: VoiceServerConfig,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl VoiceServer {
    pub fn new(config: VoiceServerConfig) -> Self {
        Self {
            state: Arc::new(Mutex::new(ServerState::Stopped)),
            running: Arc::new(AtomicBool::new(false)),
            config,
            transcription_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_error: Arc::new(Mutex::new(None)),
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

        self.running.store(true, Ordering::SeqCst);
        *self.state.lock().await = ServerState::Idle;

        info!("{LOG_PREFIX} voice server ready, listening for hotkey");

        let mut recording: Option<RecordingHandle> = None;

        while self.running.load(Ordering::SeqCst) {
            let event = tokio::select! {
                ev = hotkey_rx.recv() => {
                    match ev {
                        Some(e) => e,
                        None => {
                            warn!("{LOG_PREFIX} hotkey channel closed");
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    continue;
                }
            };

            match event {
                HotkeyEvent::Pressed => {
                    if recording.is_some() {
                        // In tap mode, second press stops recording.
                        debug!("{LOG_PREFIX} hotkey pressed while recording → stopping");
                        if let Some(handle) = recording.take() {
                            self.process_recording(handle, app_config).await;
                        }
                    } else {
                        debug!("{LOG_PREFIX} hotkey pressed → starting recording");
                        match audio_capture::start_recording() {
                            Ok(handle) => {
                                *self.state.lock().await = ServerState::Recording;
                                recording = Some(handle);
                                info!("{LOG_PREFIX} recording started");
                            }
                            Err(e) => {
                                error!("{LOG_PREFIX} failed to start recording: {e}");
                                *self.last_error.lock().await = Some(e);
                            }
                        }
                    }
                }
                HotkeyEvent::Released => {
                    // In push mode, release stops recording.
                    if let Some(handle) = recording.take() {
                        debug!("{LOG_PREFIX} hotkey released → stopping recording");
                        self.process_recording(handle, app_config).await;
                    }
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
        self.running.store(false, Ordering::SeqCst);
    }

    /// Process a completed recording: transcribe and insert text.
    async fn process_recording(&self, handle: RecordingHandle, config: &Config) {
        *self.state.lock().await = ServerState::Transcribing;

        match handle.stop().await {
            Ok(result) => {
                info!(
                    "{LOG_PREFIX} recording stopped: {:.1}s, {} bytes",
                    result.duration_secs,
                    result.wav_bytes.len()
                );

                // Minimum recording duration threshold (300ms).
                if result.duration_secs < 0.3 {
                    warn!("{LOG_PREFIX} recording too short ({:.1}s), skipping", result.duration_secs);
                    *self.state.lock().await = ServerState::Idle;
                    return;
                }

                match crate::openhuman::voice::voice_transcribe_bytes(
                    config,
                    &result.wav_bytes,
                    Some("wav".to_string()),
                    self.config.context.as_deref(),
                    self.config.skip_cleanup,
                )
                .await
                {
                    Ok(outcome) => {
                        let text = &outcome.value.text;
                        info!(
                            "{LOG_PREFIX} transcription: '{}' ({} chars)",
                            truncate_for_log(text, 80),
                            text.len()
                        );

                        if !text.trim().is_empty() {
                            if let Err(e) = text_input::insert_text(text) {
                                error!("{LOG_PREFIX} failed to insert text: {e}");
                                *self.last_error.lock().await = Some(e);
                            } else {
                                self.transcription_count.fetch_add(1, Ordering::Relaxed);
                                info!("{LOG_PREFIX} text inserted into active field");
                            }
                        } else {
                            debug!("{LOG_PREFIX} transcription was empty, nothing to insert");
                        }
                    }
                    Err(e) => {
                        error!("{LOG_PREFIX} transcription failed: {e}");
                        *self.last_error.lock().await = Some(e);
                    }
                }
            }
            Err(e) => {
                error!("{LOG_PREFIX} failed to stop recording: {e}");
                *self.last_error.lock().await = Some(e);
            }
        }

        *self.state.lock().await = ServerState::Idle;
    }
}

/// Global voice server instance, lazily initialized.
static VOICE_SERVER: once_cell::sync::OnceCell<Arc<VoiceServer>> =
    once_cell::sync::OnceCell::new();

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

/// Run the voice server standalone (blocking). Intended for CLI usage.
pub async fn run_standalone(app_config: Config, server_config: VoiceServerConfig) -> Result<(), String> {
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

fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_server_config() {
        let cfg = VoiceServerConfig::default();
        assert_eq!(cfg.hotkey, "ctrl+shift+space");
        assert_eq!(cfg.activation_mode, ActivationMode::Tap);
        assert!(!cfg.skip_cleanup);
        assert!(cfg.context.is_none());
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
            hotkey: "ctrl+shift+space".into(),
            activation_mode: ActivationMode::Tap,
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
