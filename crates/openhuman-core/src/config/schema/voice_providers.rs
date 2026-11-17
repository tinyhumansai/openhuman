//! Voice provider credential schema.
//!
//! Each entry in `Config::voice_providers` represents one configured STT or
//! TTS backend. Providers are keyed by a user-chosen `slug` (e.g. `"deepgram"`,
//! `"elevenlabs"`). The factory in `crate::voice::factory` resolves
//! routing strings against this list at runtime using the grammar
//! `"<slug>:<model>"`.
//!
//! Secrets are NOT stored on [`VoiceProviderCreds`]. They live in
//! `auth-profiles.json` under `provider:<slug>` — the same namespace as LLM
//! cloud providers, so a user who configures `openai` for both LLM and voice
//! shares one key automatically.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::cloud_providers::AuthStyle;

/// What a voice provider can do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum VoiceCapability {
    Stt,
    Tts,
    #[default]
    Both,
}

impl VoiceCapability {
    pub fn supports_stt(&self) -> bool {
        matches!(self, Self::Stt | Self::Both)
    }

    pub fn supports_tts(&self) -> bool {
        matches!(self, Self::Tts | Self::Both)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stt => "stt",
            Self::Tts => "tts",
            Self::Both => "both",
        }
    }
}

// The API-style enums live with the HTTP clients that dispatch on them
// (`tinyinference_voice::external_{stt,tts}`); the TOML shape (`rename_all = "lowercase"`) and JSON schema are unchanged.
use tinyinference_voice::external_stt::SttApiStyle;
use tinyinference_voice::external_tts::TtsApiStyle;

/// Endpoint config for one voice (STT/TTS) provider.
///
/// Mirrors [`super::cloud_providers::CloudProviderCreds`] with voice-specific
/// fields (`capability`, `stt_api_style`, `tts_api_style`, default model/voice).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceProviderCreds {
    /// Opaque stable id, e.g. `"vp_deepgram_a8c3f"`. Never shown in the UI.
    pub id: String,
    /// Routing key chosen by the user or seeded from built-in metadata.
    /// Must be unique per config and not in the reserved list.
    pub slug: String,
    /// Human-readable display label. Not used in routing.
    pub label: String,
    /// Provider base URL.
    pub endpoint: String,
    /// Authentication header style (reused from LLM cloud providers).
    pub auth_style: AuthStyle,
    /// What this provider can do: STT, TTS, or both.
    pub capability: VoiceCapability,
    /// API style for STT requests (only relevant when capability is Stt or Both).
    pub stt_api_style: SttApiStyle,
    /// API style for TTS requests (only relevant when capability is Tts or Both).
    pub tts_api_style: TtsApiStyle,
    /// Default STT model for this provider (e.g. `"nova-2"` for Deepgram).
    pub default_stt_model: Option<String>,
    /// Default TTS voice/model for this provider (e.g. `"alloy"` for OpenAI).
    pub default_tts_voice: Option<String>,
}

impl Default for VoiceProviderCreds {
    fn default() -> Self {
        Self {
            id: String::new(),
            slug: String::new(),
            label: String::new(),
            endpoint: String::new(),
            auth_style: AuthStyle::Bearer,
            capability: VoiceCapability::Both,
            stt_api_style: SttApiStyle::OpenaiAudio,
            tts_api_style: TtsApiStyle::OpenaiAudio,
            default_stt_model: None,
            default_tts_voice: None,
        }
    }
}

// ── Built-in slug metadata ──────────────────────────────────────────────────

/// Metadata for a built-in voice provider slug. Used by the frontend to
/// seed defaults when the user enables a provider — not persisted to config.
pub struct BuiltinVoiceProvider {
    pub slug: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub capability: VoiceCapability,
    pub stt_api_style: SttApiStyle,
    pub tts_api_style: TtsApiStyle,
    pub default_stt_model: Option<&'static str>,
    pub default_tts_voice: Option<&'static str>,
}

pub const BUILTIN_VOICE_PROVIDERS: &[BuiltinVoiceProvider] = &[
    BuiltinVoiceProvider {
        slug: "deepgram",
        label: "Deepgram",
        endpoint: "https://api.deepgram.com/v1",
        capability: VoiceCapability::Stt,
        stt_api_style: SttApiStyle::Deepgram,
        tts_api_style: TtsApiStyle::OpenaiAudio,
        default_stt_model: Some("nova-2"),
        default_tts_voice: None,
    },
    BuiltinVoiceProvider {
        slug: "elevenlabs",
        label: "ElevenLabs",
        endpoint: "https://api.elevenlabs.io/v1",
        capability: VoiceCapability::Both,
        stt_api_style: SttApiStyle::ElevenLabs,
        tts_api_style: TtsApiStyle::ElevenLabs,
        default_stt_model: Some("scribe_v1"),
        default_tts_voice: Some("JBFqnCBsd6RMkjVDRZzb"),
    },
    BuiltinVoiceProvider {
        slug: "openai",
        label: "OpenAI",
        endpoint: "https://api.openai.com/v1",
        capability: VoiceCapability::Both,
        stt_api_style: SttApiStyle::OpenaiAudio,
        tts_api_style: TtsApiStyle::OpenaiAudio,
        default_stt_model: Some("whisper-1"),
        default_tts_voice: Some("alloy"),
    },
];

/// Look up built-in metadata by slug.
pub fn builtin_voice_provider(slug: &str) -> Option<&'static BuiltinVoiceProvider> {
    BUILTIN_VOICE_PROVIDERS.iter().find(|p| p.slug == slug)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Reserved slugs that may not be used for user-configured voice providers.
/// These are sentinels in the voice factory's routing grammar.
pub fn is_voice_slug_reserved(s: &str) -> bool {
    matches!(
        s.trim(),
        "" | "cloud" | "openhuman" | "backend" | "whisper" | "local" | "piper"
    )
}

/// Generate a short opaque id for a new voice provider entry.
///
/// Format: `"vp_<slug>_<5 random alphanumerics>"`.
pub fn generate_voice_provider_id(slug: &str) -> String {
    super::generate_provider_id_with_prefix("vp", slug)
}

#[cfg(test)]
#[path = "voice_providers_tests.rs"]
mod tests;
