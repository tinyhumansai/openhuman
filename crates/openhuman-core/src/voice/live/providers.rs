//! Live provider catalogue, configuration and connection preparation.
//!
//! Two built-in providers, both bring-your-own-key:
//!
//! | id | reached through | credential |
//! | --- | --- | --- |
//! | `gemini` | Google directly | `provider:google` |
//! | `sarvam` | Sarvam directly | `provider:sarvam` |
//!
//! Hosted providers (`gemini-hosted`, `elevenlabs-hosted`) are not built in:
//! the hosted-backend layer registers them through [`super::registry`], where
//! they mint tickets and signed URLs against the backend, because
//! `tinyliveagents` deliberately holds no backend or credential code. With none
//! registered those ids resolve to an "unknown live provider" error.

use serde_json::json;
use tinyagents_live::tinyliveagents::gemini::GeminiLive;
use tinyagents_live::tinyliveagents::sarvam::SarvamCascade;
use tinyagents_live::tinyliveagents::{AudioFormat, LiveConfig, LiveProvider};

use super::error::LiveVoiceError;
use super::types::{LiveProviderInfo, LiveProviderKind};
use crate::config::schema::voice_live::{
    LIVE_PROVIDERS, LIVE_PROVIDER_GEMINI, LIVE_PROVIDER_SARVAM,
};
use crate::config::Config;

const LOG_PREFIX: &str = "[voice-live]";

/// Gemini Live prebuilt voices.
pub const GEMINI_VOICES: &[&str] = &[
    "Puck", "Charon", "Kore", "Fenrir", "Aoede", "Leda", "Orus", "Zephyr",
];
/// Languages Gemini Live speaks well (it understands more).
pub const GEMINI_LANGUAGES: &[&str] = &[
    "en-US", "en-IN", "en-GB", "hi-IN", "es-US", "es-ES", "fr-FR", "de-DE", "it-IT", "pt-BR",
    "ja-JP", "ko-KR", "cmn-CN", "ar-XA", "id-ID", "ru-RU", "pl-PL", "bn-IN",
];
/// Sarvam `bulbul:v3` speakers offered in the UI.
const SARVAM_SPEAKERS: &[&str] = &[
    "shubh", "priya", "aditya", "ritu", "neha", "rahul", "pooja", "rohan", "simran", "kavya",
    "amit", "dev", "ishita", "shreya", "varun", "kabir",
];
/// Languages Sarvam can both hear and speak, plus automatic detection.
const SARVAM_LANGUAGES: &[&str] = &[
    "en-IN", "hi-IN", "bn-IN", "gu-IN", "kn-IN", "ml-IN", "mr-IN", "od-IN", "pa-IN", "ta-IN",
    "te-IN", "auto",
];

/// The BYOK key slug a provider reads, if any.
pub(crate) fn key_slug(provider: &str) -> Option<&'static str> {
    match provider {
        LIVE_PROVIDER_GEMINI => Some("google"),
        LIVE_PROVIDER_SARVAM => Some("sarvam"),
        _ => None,
    }
}

/// The stored key for `slug`, or `None` when none is stored.
pub(crate) fn stored_key(slug: &str, config: &Config) -> Option<String> {
    crate::inference::provider::factory::lookup_key_for_slug(slug, config)
        .ok()
        .filter(|key| !key.trim().is_empty())
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// The provider catalogue with each provider's readiness.
pub(crate) fn provider_infos(config: &Config) -> Vec<LiveProviderInfo> {
    let mut infos = built_in_provider_infos(config);
    super::registry::merge_provider_infos(&mut infos, config);
    infos
}

fn built_in_provider_infos(config: &Config) -> Vec<LiveProviderInfo> {
    LIVE_PROVIDERS
        .iter()
        .filter(|id| key_slug(id).is_some())
        .map(|id| {
            let slug = key_slug(id);
            let (label, kind, voices, languages) = match *id {
                LIVE_PROVIDER_GEMINI => (
                    "Gemini Live (Google API key)",
                    LiveProviderKind::Byok,
                    strings(GEMINI_VOICES),
                    strings(GEMINI_LANGUAGES),
                ),
                _ => (
                    "Sarvam AI",
                    LiveProviderKind::Byok,
                    strings(SARVAM_SPEAKERS),
                    strings(SARVAM_LANGUAGES),
                ),
            };
            let configured = slug.is_some_and(|slug| stored_key(slug, config).is_some());
            LiveProviderInfo {
                id: (*id).to_string(),
                label: label.to_string(),
                kind,
                configured,
                key_slug: slug.map(str::to_string),
                voices,
                languages,
            }
        })
        .collect()
}

/// The session configuration for `provider`, from the user's settings.
pub(crate) fn live_config(
    config: &Config,
    provider: &str,
    system_instruction: &str,
    input_sample_rate: u32,
) -> LiveConfig {
    let settings = &config.voice_live;
    let mut live = LiveConfig::new();
    live.input_format = AudioFormat::pcm16(input_sample_rate);
    match provider {
        LIVE_PROVIDER_GEMINI => {
            live.system_instruction = Some(system_instruction.to_string());
            live.model = settings.gemini.model.clone();
            live.voice = settings.gemini.voice.clone();
            live.language = settings.gemini.language.clone();
            // Keep long conversations inside the context window.
            live.provider_options =
                json!({ "context_window_compression": { "slidingWindow": {} } });
        }
        LIVE_PROVIDER_SARVAM => {
            live.system_instruction = Some(system_instruction.to_string());
            live.model = settings.sarvam.model.clone();
            live.voice = settings.sarvam.speaker.clone();
            match settings.sarvam.language.as_deref() {
                Some("auto") => live.provider_options = json!({ "auto_language": true }),
                other => live.language = Some(other.unwrap_or("en-IN").to_string()),
            }
        }
        _ => {}
    }
    super::registry::configure(config, provider, system_instruction, &mut live);
    live
}

/// A provider ready to connect, with the configuration it must connect with.
pub struct PreparedProvider {
    pub provider: Box<dyn LiveProvider>,
    pub config: LiveConfig,
}

/// Resolves `provider` to something `tinyagents-live` can connect, minting a
/// backend ticket or signed URL for the hosted ones (registered, see
/// [`super::registry`]).
pub(crate) async fn prepare(
    config: &Config,
    provider: &str,
    live: LiveConfig,
) -> Result<PreparedProvider, LiveVoiceError> {
    if let Some(entry) = super::registry::lookup(provider) {
        return super::registry::prepare_with(&entry, provider, config, live).await;
    }
    match provider {
        LIVE_PROVIDER_GEMINI => {
            let key = stored_key("google", config).ok_or_else(|| {
                LiveVoiceError::not_configured("add a Google API key for Gemini Live")
            })?;
            Ok(PreparedProvider {
                provider: Box::new(GeminiLive::new(key)),
                config: live,
            })
        }
        LIVE_PROVIDER_SARVAM => {
            let key = stored_key("sarvam", config)
                .ok_or_else(|| LiveVoiceError::not_configured("add a Sarvam API key"))?;
            Ok(PreparedProvider {
                provider: Box::new(SarvamCascade::new(key)),
                config: live,
            })
        }
        other => Err(LiveVoiceError::invalid(format!(
            "unknown live provider `{other}`"
        ))),
    }
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
