//! Voice provider credential schema.
//!
//! Each entry in `Config::voice_providers` represents one configured STT or
//! TTS backend. Providers are keyed by a user-chosen `slug` (e.g. `"deepgram"`,
//! `"elevenlabs"`). The factory in `crate::openhuman::voice::factory` resolves
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

/// API style for STT requests. Different providers use incompatible request
/// shapes; the factory dispatches based on this discriminator.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SttApiStyle {
    /// OpenAI-compatible: multipart POST to `/audio/transcriptions`.
    #[default]
    OpenaiAudio,
    /// Deepgram: POST binary audio to `/listen?model=<model>`.
    Deepgram,
}

/// API style for TTS requests.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TtsApiStyle {
    /// OpenAI-compatible: POST JSON `{ model, voice, input }` to `/audio/speech`.
    #[default]
    OpenaiAudio,
    /// ElevenLabs: POST JSON `{ text, model_id }` to `/text-to-speech/<voice_id>`.
    ElevenLabs,
}

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
        stt_api_style: SttApiStyle::OpenaiAudio,
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
    matches!(s.trim(), "" | "cloud" | "openhuman" | "whisper" | "piper")
}

/// Generate a short opaque id for a new voice provider entry.
///
/// Format: `"vp_<slug>_<5 random alphanumerics>"`.
pub fn generate_voice_provider_id(slug: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut suffix = String::with_capacity(5);
    let mut seed = nanos as usize;
    for _ in 0..5 {
        suffix.push(chars[seed % chars.len()] as char);
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed = (seed >> 33) ^ seed;
    }
    let safe_slug: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(20)
        .collect();
    format!("vp_{}_{}", safe_slug, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_slugs() {
        for s in ["", " ", "cloud", "openhuman", "whisper", "piper"] {
            assert!(is_voice_slug_reserved(s), "{s:?} must be reserved");
        }
    }

    #[test]
    fn non_reserved_slugs() {
        for s in ["deepgram", "elevenlabs", "openai", "groq", "my-custom"] {
            assert!(!is_voice_slug_reserved(s), "{s:?} must not be reserved");
        }
    }

    #[test]
    fn generated_id_has_vp_prefix() {
        let id = generate_voice_provider_id("deepgram");
        assert!(id.starts_with("vp_deepgram_"), "got: {id}");
        assert_eq!(id.len(), "vp_deepgram_".len() + 5);
    }

    #[test]
    fn generated_id_sanitises_slug() {
        let id = generate_voice_provider_id("my provider!");
        assert!(id.starts_with("vp_my_provider_"), "got: {id}");
    }

    #[test]
    fn builtin_lookup_finds_known_slugs() {
        assert!(builtin_voice_provider("deepgram").is_some());
        assert!(builtin_voice_provider("elevenlabs").is_some());
        assert!(builtin_voice_provider("openai").is_some());
    }

    #[test]
    fn builtin_lookup_misses_unknown() {
        assert!(builtin_voice_provider("groq").is_none());
    }

    #[test]
    fn capability_helpers() {
        assert!(VoiceCapability::Stt.supports_stt());
        assert!(!VoiceCapability::Stt.supports_tts());
        assert!(!VoiceCapability::Tts.supports_stt());
        assert!(VoiceCapability::Tts.supports_tts());
        assert!(VoiceCapability::Both.supports_stt());
        assert!(VoiceCapability::Both.supports_tts());
    }

    #[test]
    fn default_creds_round_trips() {
        let creds = VoiceProviderCreds::default();
        let json = serde_json::to_string(&creds).unwrap();
        let back: VoiceProviderCreds = serde_json::from_str(&json).unwrap();
        assert_eq!(creds, back);
    }

    #[test]
    fn creds_with_fields_round_trips() {
        let creds = VoiceProviderCreds {
            id: "vp_deepgram_abc12".into(),
            slug: "deepgram".into(),
            label: "Deepgram".into(),
            endpoint: "https://api.deepgram.com/v1".into(),
            auth_style: AuthStyle::Bearer,
            capability: VoiceCapability::Stt,
            stt_api_style: SttApiStyle::Deepgram,
            tts_api_style: TtsApiStyle::OpenaiAudio,
            default_stt_model: Some("nova-2".into()),
            default_tts_voice: None,
        };
        let json = serde_json::to_string(&creds).unwrap();
        let back: VoiceProviderCreds = serde_json::from_str(&json).unwrap();
        assert_eq!(creds, back);
    }
}
