//! Helper functions: slug parsing, config resolution, slug-keyed provider lookup.

use log::debug;

use super::stt_providers::ExternalSttProvider;
use super::traits::{SttProvider, TtsProvider};
use super::tts_providers::ExternalTtsProvider;
use crate::openhuman::config::Config;

pub(super) const LOG_PREFIX: &str = "[voice-factory]";

// ---------------------------------------------------------------------------
// Slug / model parsing
// ---------------------------------------------------------------------------

/// Split a provider string into `(slug, model)`.
///
/// `"deepgram:nova-2"` → `("deepgram", "nova-2")`
/// `"deepgram"` → `("deepgram", "")`
pub(super) fn split_slug_model(s: &str) -> (&str, &str) {
    match s.find(':') {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => (s, ""),
    }
}

// ---------------------------------------------------------------------------
// Effective provider resolution
// ---------------------------------------------------------------------------

/// Resolve the effective STT provider string from config.
///
/// Precedence: `config.stt_provider` → `config.local_ai.stt_provider` → `"cloud"`.
pub fn effective_stt_provider(config: &Config) -> String {
    config
        .stt_provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let legacy = config.local_ai.stt_provider.as_str();
            if legacy.trim().is_empty() {
                None
            } else {
                Some(legacy)
            }
        })
        .unwrap_or("cloud")
        .to_string()
}

/// Resolve the effective TTS provider string from config.
///
/// Precedence: `config.tts_provider` → `config.local_ai.tts_provider` → `"cloud"`.
pub fn effective_tts_provider(config: &Config) -> String {
    config
        .tts_provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let legacy = config.local_ai.tts_provider.as_str();
            if legacy.trim().is_empty() {
                None
            } else {
                Some(legacy)
            }
        })
        .unwrap_or("cloud")
        .to_string()
}

// ---------------------------------------------------------------------------
// Slug-keyed provider creation
// ---------------------------------------------------------------------------

/// Create an STT provider by looking up a slug in `config.voice_providers`.
pub(super) fn create_stt_provider_by_slug(
    slug: &str,
    model: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn SttProvider>> {
    let entry = config
        .voice_providers
        .iter()
        .find(|p| p.slug == slug)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no voice provider with slug '{}' found in voice_providers",
                slug
            )
        })?;

    if !entry.capability.supports_stt() {
        return Err(anyhow::anyhow!(
            "voice provider '{}' does not support STT (capability: {})",
            slug,
            entry.capability.as_str()
        ));
    }

    let effective_model = if model.trim().is_empty() {
        entry.default_stt_model.as_deref().unwrap_or("default")
    } else {
        model
    };

    let api_key = crate::openhuman::inference::provider::factory::lookup_key_for_slug(slug, config)
        .unwrap_or_default();

    debug!(
        "{LOG_PREFIX} creating external STT provider slug={slug} model={effective_model} \
         endpoint={} key_present={}",
        entry.endpoint,
        !api_key.is_empty()
    );

    Ok(Box::new(ExternalSttProvider::new(
        slug,
        effective_model,
        &entry.endpoint,
        api_key,
        entry.stt_api_style,
    )))
}

/// Create a TTS provider by looking up a slug in `config.voice_providers`.
pub(super) fn create_tts_provider_by_slug(
    slug: &str,
    voice: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn TtsProvider>> {
    let entry = config
        .voice_providers
        .iter()
        .find(|p| p.slug == slug)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no voice provider with slug '{}' found in voice_providers",
                slug
            )
        })?;

    if !entry.capability.supports_tts() {
        return Err(anyhow::anyhow!(
            "voice provider '{}' does not support TTS (capability: {})",
            slug,
            entry.capability.as_str()
        ));
    }

    let effective_voice = if voice.trim().is_empty() {
        entry.default_tts_voice.as_deref().unwrap_or("default")
    } else {
        voice
    };

    let api_key = crate::openhuman::inference::provider::factory::lookup_key_for_slug(slug, config)
        .unwrap_or_default();

    debug!(
        "{LOG_PREFIX} creating external TTS provider slug={slug} voice={effective_voice} \
         endpoint={} key_present={}",
        entry.endpoint,
        !api_key.is_empty()
    );

    Ok(Box::new(ExternalTtsProvider::new(
        slug,
        effective_voice,
        &entry.endpoint,
        api_key,
        entry.tts_api_style,
    )))
}

// ---------------------------------------------------------------------------
// Low-level utilities
// ---------------------------------------------------------------------------

pub(super) fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("[voice-factory] base64 decode error: {e}"))
}

pub(super) fn extension_for_mime(mime: &str) -> &str {
    match mime {
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" => "ogg",
        "audio/webm" => "webm",
        "audio/flac" => "flac",
        "audio/mp4" | "audio/m4a" => "m4a",
        _ => "wav",
    }
}
