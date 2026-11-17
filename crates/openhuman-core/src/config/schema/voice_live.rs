//! Live voice agent settings (`[voice_live]`).
//!
//! Which live provider the voice agent (Tiny's realtime mode) uses, and each
//! provider's model, voice and language. Secrets never live here: BYOK keys
//! are stored in `auth-profiles.json` under `provider:google` (Gemini) and
//! `provider:sarvam`, through the same `auth.store_provider_credentials` path
//! the other providers use.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Gemini Live through the TinyHumans backend relay (no user key).
pub const LIVE_PROVIDER_GEMINI_HOSTED: &str = "gemini-hosted";
/// The TinyHumans-hosted ElevenLabs agent (signed URL from the backend).
pub const LIVE_PROVIDER_ELEVENLABS_HOSTED: &str = "elevenlabs-hosted";
/// Gemini Live with the user's own Google API key.
pub const LIVE_PROVIDER_GEMINI: &str = "gemini";
/// Sarvam AI (STT → chat → TTS) with the user's own key.
pub const LIVE_PROVIDER_SARVAM: &str = "sarvam";

/// Every live provider id, in display order.
pub const LIVE_PROVIDERS: &[&str] = &[
    LIVE_PROVIDER_GEMINI_HOSTED,
    LIVE_PROVIDER_ELEVENLABS_HOSTED,
    LIVE_PROVIDER_GEMINI,
    LIVE_PROVIDER_SARVAM,
];

/// Gemini Live options (hosted and direct).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GeminiLiveSettings {
    /// Live model id; unset uses the library default.
    #[serde(default)]
    pub model: Option<String>,
    /// Prebuilt voice name (`Puck`, `Kore`, ...).
    #[serde(default)]
    pub voice: Option<String>,
    /// BCP-47 language for speech output.
    #[serde(default)]
    pub language: Option<String>,
}

/// Sarvam options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SarvamLiveSettings {
    /// BCP-47 language (`en-IN`, `hi-IN`, ...); unset uses `en-IN`. `auto`
    /// detects the language and answers in it.
    #[serde(default)]
    pub language: Option<String>,
    /// `bulbul:v3` speaker (`shubh`, `priya`, ...).
    #[serde(default)]
    pub speaker: Option<String>,
    /// Chat model; unset uses `sarvam-105b-conversations`.
    #[serde(default)]
    pub model: Option<String>,
}

/// ElevenLabs options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ElevenLabsLiveSettings {
    /// Voice override; unset uses the mascot voice / the agent's own.
    #[serde(default)]
    pub voice_id: Option<String>,
}

/// `[voice_live]` — live voice agent settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LiveVoiceConfig {
    /// The provider Tiny uses when the caller names none.
    #[serde(default = "default_provider")]
    pub default_provider: String,
    #[serde(default)]
    pub gemini: GeminiLiveSettings,
    #[serde(default)]
    pub sarvam: SarvamLiveSettings,
    #[serde(default)]
    pub elevenlabs: ElevenLabsLiveSettings,
}

fn default_provider() -> String {
    LIVE_PROVIDER_GEMINI_HOSTED.to_string()
}

impl Default for LiveVoiceConfig {
    fn default() -> Self {
        Self {
            default_provider: default_provider(),
            gemini: GeminiLiveSettings::default(),
            sarvam: SarvamLiveSettings::default(),
            elevenlabs: ElevenLabsLiveSettings::default(),
        }
    }
}

/// Whether `id` names a live provider.
pub fn is_live_provider(id: &str) -> bool {
    LIVE_PROVIDERS.contains(&id)
}

#[cfg(test)]
#[path = "voice_live_tests.rs"]
mod tests;
