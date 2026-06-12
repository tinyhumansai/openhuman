//! Factory entry points: `create_stt_provider`, `create_tts_provider`, defaults, and constants.

use std::sync::Arc;

use log::debug;

use super::helpers::{
    create_stt_provider_by_slug, create_tts_provider_by_slug, split_slug_model, LOG_PREFIX,
};
use super::stt_providers::{CloudSttProvider, WhisperSttProvider};
use super::traits::{SttProvider, TtsProvider};
use super::tts_providers::{CloudTtsProvider, PiperTtsProvider};
use crate::openhuman::config::Config;

/// Default Whisper model. `whisper-large-v3-turbo` is the recommended ship
/// default — best accuracy-to-latency tradeoff in the Whisper family (5×
/// faster than `large-v3` with comparable WER on English). Users on lower-
/// spec hardware can drop down to `medium` / `small` / `base` / `tiny` via
/// the install presets.
pub const DEFAULT_WHISPER_MODEL: &str = "whisper-large-v3-turbo";

/// Default Piper voice — `en_US-lessac-medium`, matches
/// [`super::super::local_ai::model_ids::effective_tts_voice_id`].
pub const DEFAULT_PIPER_VOICE: &str = "en_US-lessac-medium";

/// Whisper install presets (size tiers exposed to the installer UI).
/// Mirrors the Ollama model installer surface: each entry is `(id, label)`.
pub const WHISPER_MODEL_PRESETS: &[(&str, &str)] = &[
    ("tiny", "Tiny (39 MB, fastest)"),
    ("base", "Base (74 MB)"),
    ("small", "Small (244 MB)"),
    ("medium", "Medium (769 MB, recommended)"),
    ("large-v3-turbo", "Large v3 Turbo (1.5 GB, best accuracy)"),
];

/// Creates a speech-to-text provider based on the specified name and model.
///
/// Supported provider names:
/// - `"cloud"` → backend Whisper proxy — default, preferred for laptops
///   without local models
/// - `"whisper"` → local whisper.cpp via `WHISPER_BIN` (or in-process
///   `whisper-rs` when configured)
///
/// Returns an error for unrecognised provider names so configuration
/// mistakes surface immediately rather than silently degrading.
///
/// The factory does not eagerly resolve the binary — `WhisperSttProvider`
/// looks up `WHISPER_BIN` lazily inside `transcribe()` so a misconfigured
/// install fails at use-time with a clear error message instead of at
/// startup.
pub fn create_stt_provider(
    provider: &str,
    model: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn SttProvider>> {
    debug!("{LOG_PREFIX} create_stt_provider provider={provider} model={model}");
    let model = if model.trim().is_empty() {
        DEFAULT_WHISPER_MODEL
    } else {
        model
    };
    match provider.trim() {
        "cloud" | "openhuman" => Ok(Box::new(CloudSttProvider::new(
            super::super::cloud_transcribe_default_model(),
        ))),
        "whisper" => Ok(Box::new(WhisperSttProvider::new(model))),
        other => {
            let (slug, slug_model) = split_slug_model(other);
            let effective_model = if slug_model.is_empty() {
                model
            } else {
                slug_model
            };
            create_stt_provider_by_slug(slug, effective_model, config)
        }
    }
}

/// Creates a text-to-speech provider based on the specified name and voice.
///
/// Supported provider names:
/// - `"cloud"` → backend ElevenLabs proxy with viseme alignment
/// - `"piper"` → local Piper subprocess via `PIPER_BIN`
///
/// Kokoro is **not** implemented in this cut — the integration shipped with
/// Piper because `PIPER_BIN` is already reserved in `.env.example` and the
/// runtime contract (subprocess + `.onnx` model) is simpler. Adding Kokoro
/// later is straightforward: add a new branch here and a `local_speech_kokoro`
/// sibling module.
pub fn create_tts_provider(
    provider: &str,
    voice: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn TtsProvider>> {
    debug!("{LOG_PREFIX} create_tts_provider provider={provider} voice={voice}");
    let voice = if voice.trim().is_empty() {
        DEFAULT_PIPER_VOICE
    } else {
        voice
    };
    match provider.trim() {
        "cloud" | "openhuman" => Ok(Box::new(CloudTtsProvider::new(if voice.is_empty() {
            None
        } else {
            Some(voice.to_string())
        }))),
        "piper" => Ok(Box::new(PiperTtsProvider::new(voice))),
        other => {
            let (slug, slug_voice) = split_slug_model(other);
            let effective_voice = if slug_voice.is_empty() {
                voice
            } else {
                slug_voice
            };
            create_tts_provider_by_slug(slug, effective_voice, config)
        }
    }
}

/// Returns a thread-safe default STT provider (cloud). Used by callers that
/// can't easily plumb a `Config` reference but still need a sensible default.
pub fn default_stt_provider() -> Arc<dyn SttProvider> {
    Arc::new(CloudSttProvider::new(
        super::super::cloud_transcribe_default_model(),
    ))
}

/// Returns a thread-safe default TTS provider (cloud).
pub fn default_tts_provider() -> Arc<dyn TtsProvider> {
    Arc::new(CloudTtsProvider::new(None))
}
