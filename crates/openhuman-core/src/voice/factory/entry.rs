//! Factory entry points: `create_stt_provider`, `create_tts_provider`, defaults, and constants.

use log::debug;

use super::helpers::{
    create_stt_provider_by_slug, create_tts_provider_by_slug, split_slug_model, LOG_PREFIX,
};
use super::registry::{stt_factory, tts_factory};
use super::traits::{SttProvider, TtsProvider};
use super::tts_providers::PiperTtsProvider;
use crate::config::Config;

/// Default STT model id for a hosted transcription proxy.
/// Third-party providers use the `default_stt_model` configured in their
/// registry entry when callers leave the model empty.
pub const DEFAULT_STT_MODEL: &str = "whisper-v1";

/// Default Piper voice — `en_US-lessac-medium`, matches
/// [`super::super::local_ai::model_ids::effective_tts_voice_id`].
pub const DEFAULT_PIPER_VOICE: &str = "en_US-lessac-medium";

/// Creates a speech-to-text provider from a routing string.
///
/// Supported provider names:
/// - A slug registered with the voice registry (see [`super::registry`]) wins.
///   The hosted `"cloud"` / `"openhuman"` / `"backend"` slugs are registered by
///   the hosted-backend layer, not built into the core; with none registered
///   they answer the `BACKEND_UNAVAILABLE:` sentinel.
/// - `"<slug>"` / `"<slug>:<model>"` → a third-party STT API resolved against
///   `config.voice_providers` (e.g. `"elevenlabs"`, `"openai:whisper-1"`,
///   `"deepgram:nova-2"`).
///
/// **There is no local branch.** `"whisper"` and `"local"` used to select the
/// bundled whisper.cpp engine; that engine is gone, and both strings now fall
/// through to the slug lookup and error by name. `config::migrations` rewrites
/// persisted configs so a user never reaches that error, but the factory does
/// not silently remap them — an unmigrated string is a real misconfiguration
/// and a silent fallback would hide it.
///
/// Returns an error for unrecognised provider names so configuration mistakes
/// surface immediately rather than degrading quietly to a different provider.
pub fn create_stt_provider(
    provider: &str,
    model: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn SttProvider>> {
    debug!("{LOG_PREFIX} create_stt_provider provider={provider} model={model}");
    // A registered provider (see `registry`) wins over the slug lookup.
    let (registered_slug, registered_model) = split_slug_model(provider.trim());
    if let Some(factory) = stt_factory(registered_slug) {
        let effective_model = if registered_model.is_empty() {
            model.trim()
        } else {
            registered_model
        };
        debug!("{LOG_PREFIX} create_stt_provider using registered provider slug={registered_slug}");
        return factory(effective_model, config);
    }
    let (slug, slug_model) = split_slug_model(provider.trim());
    let effective_model = if slug_model.is_empty() {
        model.trim()
    } else {
        slug_model
    };
    create_stt_provider_by_slug(slug, effective_model, config)
}

/// Creates a text-to-speech provider based on the specified name and voice.
///
/// Supported provider names:
/// - `"cloud"` / `"openhuman"` → a registered hosted provider (see
///   [`super::registry`]); the core has no built-in one
/// - `"piper"` → local Piper subprocess via `PIPER_BIN`
///
/// Kokoro is **not** implemented in this cut — the integration shipped with
/// Piper because `PIPER_BIN` is already reserved in `.env.example` and the
/// runtime contract (subprocess + `.onnx` model) is simpler. Adding Kokoro
/// later is straightforward: add a new branch here and a `local_speech_kokoro`
/// sibling module.
///
/// **The empty-voice fallback is per-provider and must stay that way.**
/// [`DEFAULT_PIPER_VOICE`] is a Piper model id and is meaningful *only* to the
/// Piper branch. This function previously coerced an empty `voice` to that
/// constant before the match, which handed `en_US-lessac-medium` to every
/// provider: the cloud branch forwarded it as `voice_id` to the backend
/// ElevenLabs speech proxy, which rejects it with
/// **400 Bad Request** — breaking the Settings → Voice "Test" button and every
/// cloud TTS reply (#5355). It also pre-empted each external provider's
/// configured `default_tts_voice`, since the Piper id was never empty by the
/// time [`create_tts_provider_by_slug`] looked at it.
///
/// Callers deliberately pass an empty `voice` to mean "this provider picks its
/// own default" (`handle_voice_test_provider`, `handle_voice_reply_synthesize`,
/// `handle_voice_tts_dispatch`) — each of those already
/// guards the Piper default at its own level, and that guard is only effective
/// if this function honours the empty string.
pub fn create_tts_provider(
    provider: &str,
    voice: &str,
    config: &Config,
) -> anyhow::Result<Box<dyn TtsProvider>> {
    debug!("{LOG_PREFIX} create_tts_provider provider={provider} voice={voice}");
    let provider = provider.trim();
    let resolved = resolve_tts_voice(provider, voice);
    debug!(
        "{LOG_PREFIX} create_tts_provider resolved provider={provider} voice={}",
        resolved.unwrap_or("<provider default>")
    );
    // A registered provider (see `registry`) wins over the built-in routes.
    let (registered_slug, _) = split_slug_model(provider);
    if let Some(factory) = tts_factory(registered_slug) {
        debug!("{LOG_PREFIX} create_tts_provider using registered provider slug={registered_slug}");
        return factory(resolved, config);
    }
    match provider {
        "piper" => Ok(Box::new(PiperTtsProvider::new(
            // `resolve_tts_voice` guarantees `Some` for piper.
            resolved.unwrap_or(DEFAULT_PIPER_VOICE),
        ))),
        other => {
            let (slug, _) = split_slug_model(other);
            create_tts_provider_by_slug(slug, resolved.unwrap_or(""), config)
        }
    }
}

/// Resolve the voice a TTS provider should be constructed with.
///
/// `None` means "let the provider pick its own default": the cloud branch
/// omits `voice_id` from the backend request body, and the slug branch falls
/// through to the registry entry's `default_tts_voice`.
///
/// Both arguments are trimmed here, so the function is safe to call with raw
/// caller input: an untrimmed `"  cloud  "` would otherwise match neither
/// literal arm and fall through to the slug branch, silently treating a known
/// provider as an unknown one. For the slug form (`"openai:shimmer"`), a voice
/// in the suffix wins over the `voice` argument, matching the STT
/// model-resolution order in [`create_stt_provider`].
pub(super) fn resolve_tts_voice<'a>(provider: &'a str, voice: &'a str) -> Option<&'a str> {
    let voice = voice.trim();
    match provider.trim() {
        // `DEFAULT_PIPER_VOICE` is a Piper model id and is meaningful to this
        // branch only — never let it escape to another provider.
        "piper" => Some(if voice.is_empty() {
            DEFAULT_PIPER_VOICE
        } else {
            voice
        }),
        other => {
            let (_, slug_voice) = split_slug_model(other);
            if !slug_voice.is_empty() {
                Some(slug_voice)
            } else if !voice.is_empty() {
                Some(voice)
            } else {
                None
            }
        }
    }
}
