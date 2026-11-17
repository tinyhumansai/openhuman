//! Wire types for live voice sessions: RPC payloads and WebSocket frames.

use serde::{Deserialize, Deserializer, Serialize};

pub use crate::config::schema::voice_live::{
    ElevenLabsLiveSettings, GeminiLiveSettings, LiveVoiceConfig, SarvamLiveSettings,
};

/// How a provider is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveProviderKind {
    /// Through the TinyHumans backend; no user key.
    Hosted,
    /// With the user's own API key.
    Byok,
}

/// One live provider, as the settings UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveProviderInfo {
    /// Stable id (`gemini-hosted`, `elevenlabs-hosted`, `gemini`, `sarvam`).
    pub id: String,
    /// Display label.
    pub label: String,
    /// Hosted or bring-your-own-key.
    pub kind: LiveProviderKind,
    /// Whether the provider can be used right now (a key is stored, or the
    /// backend is reachable with a credential).
    pub configured: bool,
    /// The `provider:<slug>` key a BYOK provider reads, if any.
    pub key_slug: Option<String>,
    /// Voices the provider offers.
    pub voices: Vec<String>,
    /// Languages the provider speaks.
    pub languages: Vec<String>,
}

/// `voice.live_providers` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveProvidersResponse {
    pub default_provider: String,
    pub providers: Vec<LiveProviderInfo>,
}

/// `voice.live_settings_set` params: any subset of the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSettingsPatch {
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub gemini: Option<GeminiLiveSettingsPatch>,
    #[serde(default)]
    pub sarvam: Option<SarvamLiveSettingsPatch>,
    #[serde(default)]
    pub elevenlabs: Option<ElevenLabsLiveSettingsPatch>,
}

/// A provider patch distinguishes an omitted field from an explicit `null`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiLiveSettingsPatch {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub model: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub voice: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub language: Option<Option<String>>,
}

/// A provider patch distinguishes an omitted field from an explicit `null`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SarvamLiveSettingsPatch {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub language: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub speaker: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub model: Option<Option<String>>,
}

/// A provider patch distinguishes an omitted field from an explicit `null`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElevenLabsLiveSettingsPatch {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub voice_id: Option<Option<String>>,
}

fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

/// `voice.live_test_provider` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTestResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// A JSON frame the client sends on `/ws/live-voice`. Audio travels as binary
/// frames (PCM16 little-endian mono at `input_sample_rate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// Opens the session; must be the first frame.
    Start {
        #[serde(default)]
        provider: Option<String>,
        #[serde(default)]
        thread_id: Option<String>,
        /// The UI client approval prompts should be routed to.
        #[serde(default)]
        client_id: Option<String>,
        #[serde(default = "default_input_rate")]
        input_sample_rate: u32,
    },
    /// A typed user message.
    Text { text: String },
    /// Stop the agent's current reply.
    Interrupt,
    /// End the session.
    Stop,
}

fn default_input_rate() -> u32 {
    16_000
}

/// Who a transcript belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    User,
    Agent,
}

/// A JSON frame the core sends. Agent audio travels as binary frames
/// (PCM16 little-endian mono at `output_sample_rate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Ready {
        session_id: Option<String>,
        provider: String,
        output_sample_rate: u32,
        thread_id: Option<String>,
    },
    Transcript {
        role: TranscriptRole,
        text: String,
        #[serde(rename = "final")]
        is_final: bool,
    },
    ToolStarted {
        call_id: String,
        name: String,
    },
    ToolFinished {
        call_id: String,
        name: String,
        ok: bool,
        cancelled: bool,
    },
    Interrupted,
    TurnComplete,
    Error {
        code: String,
        message: String,
        fatal: bool,
    },
    Closed {
        reason: String,
    },
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
