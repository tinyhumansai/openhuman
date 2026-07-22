//! Disabled-voice facade.
//!
//! Compiled only when the `voice` Cargo feature is OFF (see the gate in
//! [`super`]). It mirrors the subset of the real `voice` public surface that
//! always-on / other-gated callers depend on, with no-op / disabled-error
//! bodies so the crate still compiles, boots, and serves `/rpc` without the
//! voice + audio_toolkit domains.
//!
//! The signatures here MUST match the real ones exactly (return types included).
//! The disabled build
//! (`cargo check --no-default-features --features "<all-but-voice>"`) is the
//! only thing that catches drift — if a real signature changes, update the
//! mirror below until that build is green again.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Error text returned by every disabled-path operation that must yield a
/// `Result`. Shared so callers/log-greps see one stable string.
const DISABLED_MSG: &str = "voice feature disabled at compile time";

// ---------------------------------------------------------------------------
// Provider factory surface (mirrors `factory::*` re-exported at the voice root)
// ---------------------------------------------------------------------------

/// Common STT result shape. Mirrors [`super::factory::SttResult`] (real build).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttResult {
    pub text: String,
    /// Lowercase provider id — kept for wire-shape parity with the real type.
    pub provider: String,
}

/// Speech-to-text provider abstraction. Object-safe (via `async_trait`) so
/// `Box<dyn SttProvider>` remains nameable at call sites; no concrete
/// implementation exists when voice is compiled out.
#[async_trait]
pub trait SttProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String>;
}

/// Resolve the effective STT provider string. With voice disabled the value is
/// never used to build a real provider; the config-independent default keeps
/// logging/telemetry callers seeing a sensible string.
pub fn effective_stt_provider(_config: &Config) -> String {
    "cloud".to_string()
}

/// Always errors: no STT provider can be constructed when voice is compiled
/// out. Callers `?`-propagate the error, so the boxed provider is never used.
pub fn create_stt_provider(
    _provider: &str,
    _model: &str,
    _config: &Config,
) -> anyhow::Result<Box<dyn SttProvider>> {
    Err(anyhow::anyhow!(DISABLED_MSG))
}

// ---------------------------------------------------------------------------
// Event bus surface (mirrors `bus::publish_ptt_transcript_committed`)
// ---------------------------------------------------------------------------

/// No-op: with voice disabled there is no PTT dictation path to announce.
pub fn publish_ptt_transcript_committed(
    _thread_id: String,
    _session_id: u64,
    _text_len: usize,
    _held_ms: u64,
    _finalized_by_watchdog: bool,
) {
    log::debug!("[voice-stub] publish_ptt_transcript_committed ignored (voice disabled)");
}

// ---------------------------------------------------------------------------
// cli::run_standalone_subcommand (kept registered as a CLI adapter)
// ---------------------------------------------------------------------------

pub mod cli {
    /// Disabled: the standalone dictation server needs the voice stack. Returns
    /// an error so `openhuman voice` reports the feature is off instead of the
    /// subcommand silently disappearing from the CLI registry.
    pub fn run_standalone_subcommand(_args: &[String]) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(super::DISABLED_MSG))
    }
}

// ---------------------------------------------------------------------------
// server::{start_if_enabled, try_global_server}
// ---------------------------------------------------------------------------

pub mod server {
    use std::sync::Arc;

    use crate::openhuman::config::Config;

    /// Opaque handle; never actually constructed when voice is disabled, but
    /// kept nameable so `Option<Arc<VoiceServer>>` call sites type-check.
    pub struct VoiceServer;

    impl VoiceServer {
        /// No-op stop — unreachable (no server is ever created).
        pub async fn stop(&self) {}
    }

    /// No-op: there is no voice server to start.
    pub async fn start_if_enabled(_config: &Config) {}

    /// Always `None`: no global voice server exists.
    pub fn try_global_server() -> Option<Arc<VoiceServer>> {
        None
    }
}

// ---------------------------------------------------------------------------
// dictation_listener::{start_if_enabled, stop, subscribe_*}
// ---------------------------------------------------------------------------

pub mod dictation_listener {
    use once_cell::sync::Lazy;
    use serde::Serialize;
    use tokio::sync::broadcast;

    use crate::openhuman::config::Config;

    /// Mirrors the real `DictationEvent` wire shape so the Socket.IO bridge's
    /// `serde_json::to_value(&event)` + `event.event_type` access type-check.
    #[derive(Debug, Clone, Serialize)]
    pub struct DictationEvent {
        #[serde(rename = "type")]
        pub event_type: String,
        pub hotkey: String,
        pub activation_mode: String,
    }

    // Senders are kept alive for the process lifetime so subscribers park
    // (never receive) rather than seeing an immediate `Closed`, matching the
    // real always-open broadcast bus. Nothing is ever published.
    static DICTATION_BUS: Lazy<broadcast::Sender<DictationEvent>> =
        Lazy::new(|| broadcast::channel(1).0);
    static TRANSCRIPTION_BUS: Lazy<broadcast::Sender<String>> =
        Lazy::new(|| broadcast::channel(1).0);

    /// Subscribe to (never-emitting) dictation events.
    pub fn subscribe_dictation_events() -> broadcast::Receiver<DictationEvent> {
        DICTATION_BUS.subscribe()
    }

    /// Subscribe to (never-emitting) transcription results.
    pub fn subscribe_transcription_results() -> broadcast::Receiver<String> {
        TRANSCRIPTION_BUS.subscribe()
    }

    /// No-op: no hotkey listener is started.
    pub async fn start_if_enabled(_config: &Config) {}

    /// No-op: nothing to stop.
    pub fn stop() {}
}

// ---------------------------------------------------------------------------
// always_on::{start_if_enabled, stop}
// ---------------------------------------------------------------------------

pub mod always_on {
    use crate::openhuman::config::Config;

    /// No-op: always-on listening does not exist when voice is disabled.
    pub async fn start_if_enabled(_config: &Config) {}

    /// No-op: nothing to stop.
    pub fn stop() {}
}

// ---------------------------------------------------------------------------
// streaming::handle_dictation_ws (re-exported from inference::voice in real)
// ---------------------------------------------------------------------------

// axum-only, and its sole caller (`core::jsonrpc::dictation_ws_handler`) is
// gated the same way, so the stub's dictation-WS surface is exclusive to the
// `http-server` feature too (#5048): voice-OFF + http-server-OFF needs no
// `voice::streaming` at all.
#[cfg(feature = "http-server")]
pub mod streaming {
    use std::sync::Arc;

    use axum::extract::ws::WebSocket;

    use crate::openhuman::config::Config;

    /// Drop the upgraded socket immediately — there is nothing to transcribe.
    pub async fn handle_dictation_ws(_socket: WebSocket, _config: Arc<Config>) {}
}

// ---------------------------------------------------------------------------
// reply_speech::{synthesize_reply, ReplySpeechOptions, ReplySpeechResult, ...}
// ---------------------------------------------------------------------------

pub mod reply_speech {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use crate::openhuman::config::Config;
    use crate::rpc::RpcOutcome;

    /// One frame on the viseme timeline. Mirrors the real type.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct VisemeFrame {
        pub viseme: String,
        pub start_ms: u64,
        pub end_ms: u64,
    }

    /// Char-level timing frame. Mirrors the real type.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct AlignmentFrame {
        pub char: String,
        pub start_ms: u64,
        pub end_ms: u64,
    }

    /// Normalized TTS response. Mirrors the real type.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ReplySpeechResult {
        pub audio_base64: String,
        pub audio_mime: String,
        pub visemes: Vec<VisemeFrame>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub alignment: Option<Vec<AlignmentFrame>>,
    }

    /// Caller-tunable knobs. Mirrors the real type (fields + `Default`).
    #[derive(Debug, Default, Clone)]
    pub struct ReplySpeechOptions {
        pub voice_id: Option<String>,
        pub model_id: Option<String>,
        pub output_format: Option<String>,
        pub voice_settings: Option<Value>,
    }

    /// Disabled: reply-speech synthesis is unavailable when voice is compiled
    /// out. Callers log the error and skip the spoken reply.
    pub async fn synthesize_reply(
        _config: &Config,
        _text: &str,
        _opts: &ReplySpeechOptions,
    ) -> Result<RpcOutcome<ReplySpeechResult>, String> {
        Err(super::DISABLED_MSG.to_string())
    }
}

// ---------------------------------------------------------------------------
// cloud_transcribe::{transcribe_cloud, CloudTranscribeOptions, ...}
// (re-exported from inference::voice in the real build)
// ---------------------------------------------------------------------------

pub mod cloud_transcribe {
    use serde::{Deserialize, Serialize};

    use crate::openhuman::config::Config;
    use crate::rpc::RpcOutcome;

    /// Caller-tunable knobs. Mirrors the real type (fields + `Default`).
    #[derive(Debug, Default, Clone)]
    pub struct CloudTranscribeOptions {
        pub model: Option<String>,
        pub language: Option<String>,
        pub mime_type: Option<String>,
        pub file_name: Option<String>,
    }

    /// Transcription result. Mirrors the real type.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CloudTranscribeResult {
        pub text: String,
    }

    /// Disabled: cloud STT is unavailable when voice is compiled out.
    pub async fn transcribe_cloud(
        _config: &Config,
        _audio_base64: &str,
        _opts: &CloudTranscribeOptions,
    ) -> Result<RpcOutcome<CloudTranscribeResult>, String> {
        Err(super::DISABLED_MSG.to_string())
    }
}
