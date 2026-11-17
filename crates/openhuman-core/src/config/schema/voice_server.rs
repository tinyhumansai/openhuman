//! Voice server configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which speech-to-text engine transcribes audio.
///
/// Replaces the removed local whisper.cpp engine: STT is a hosted round trip
/// now, so the only meaningful choice is *whose* endpoint runs it. Each variant
/// maps onto the existing [`crate::voice::factory`] routing grammar
/// — no new HTTP client was added for any of them:
///
/// | Variant      | Routing string | Client                                     |
/// |--------------|----------------|--------------------------------------------|
/// | `Backend`    | `"cloud"`      | `CloudSttProvider` (OpenHuman backend proxy) |
/// | `Elevenlabs` | `"elevenlabs"` | `ExternalSttProvider` via the `elevenlabs` `voice_providers` entry |
/// | `Openai`     | `"openai"`     | `ExternalSttProvider` via the `openai` `voice_providers` entry |
///
/// The two third-party variants need a matching `voice_providers` entry (seeded
/// from `BUILTIN_VOICE_PROVIDERS`) plus an API key in `auth-profiles.json`;
/// without one the factory errors by name rather than silently falling back, so
/// a misconfiguration is visible instead of billing the wrong account.
///
/// `Backend` is the default and needs no user setup — it rides the signed-in
/// session token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SttEngine {
    /// OpenHuman's hosted transcription proxy. Default.
    #[default]
    Backend,
    /// ElevenLabs Scribe, called directly with the user's own key.
    Elevenlabs,
    /// OpenAI audio transcriptions, called directly with the user's own key.
    Openai,
}

impl SttEngine {
    /// The voice-factory routing string this engine resolves to.
    ///
    /// `Backend` maps to the pre-existing `"cloud"` sentinel rather than to its
    /// own serde name: the routing grammar already had that string, and minting
    /// a second spelling for the same provider would mean two values to keep in
    /// sync in `create_stt_provider`.
    pub fn provider_string(self) -> &'static str {
        match self {
            Self::Backend => "cloud",
            Self::Elevenlabs => "elevenlabs",
            Self::Openai => "openai",
        }
    }

    /// Parse a persisted/RPC-supplied engine name. Case-insensitive, and
    /// tolerant of the `"cloud"` / `"openhuman"` aliases the routing grammar
    /// already used for the backend proxy.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "backend" | "cloud" | "openhuman" => Some(Self::Backend),
            "elevenlabs" => Some(Self::Elevenlabs),
            "openai" => Some(Self::Openai),
            _ => None,
        }
    }
}

/// Persisted STT routing fallback for voice providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct VoiceServerConfig {
    /// Which hosted engine transcribes audio. See [`SttEngine`]. An explicit
    /// `stt_provider` routing string (top-level or legacy `local_ai`) still
    /// wins — this field is what the `"cloud"`/unset case resolves to.
    #[serde(default)]
    pub stt_engine: SttEngine,
}

#[cfg(test)]
#[path = "voice_server_tests.rs"]
mod tests;
