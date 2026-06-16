//! Provider traits and shared result types for STT / TTS dispatch.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::super::reply_speech::ReplySpeechResult;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// Shared result type
// ---------------------------------------------------------------------------

/// Common shape both STT branches return after dispatch. Keeps the wire
/// contract identical regardless of provider — the UI only sees `text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttResult {
    pub text: String,
    /// Lowercase provider id (`"cloud"`, `"whisper"`) — exposed on the wire
    /// so the renderer can show the user which path actually ran.
    pub provider: String,
}

// ---------------------------------------------------------------------------
// Provider traits
// ---------------------------------------------------------------------------

/// Speech-to-text provider abstraction. Cloud (backend proxy) and Whisper
/// (local subprocess / in-process) both implement this; the factory hands
/// the caller a boxed trait object.
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Stable identifier used in logs and config (`"cloud"`, `"whisper"`).
    fn name(&self) -> &'static str;

    /// Transcribe a single base64-encoded audio blob.
    ///
    /// `mime_type` and `file_name` are hints; providers that don't care
    /// may ignore them. `language` is BCP-47 (`"en"`, `"es"`); pass `None`
    /// to let the provider auto-detect.
    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String>;
}

/// Text-to-speech provider abstraction. Cloud returns rich viseme alignment
/// (used by the mascot lip-sync); Piper returns audio only and the caller
/// derives a flat viseme timeline downstream.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Synthesize speech for `text`. Returns the same envelope shape as
    /// `voice.reply_synthesize` so the renderer can swap providers without
    /// branching on the response.
    async fn synthesize(
        &self,
        config: &Config,
        text: &str,
        voice: Option<&str>,
    ) -> Result<RpcOutcome<ReplySpeechResult>, String>;
}
