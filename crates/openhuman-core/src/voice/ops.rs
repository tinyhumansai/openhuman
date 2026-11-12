//! Voice domain business logic — hosted STT and local piper TTS.
//!
//! Each public function follows the `Outcome<T>` pattern used by other
//! domain modules (billing, health, etc.).

use log::{debug, warn};
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use crate::config::Config;
use crate::core::Outcome;
use crate::inference::host_runtime as local_ai;
use tinyinference_local::models as model_ids;

use super::factory::{create_stt_provider, effective_stt_provider, effective_tts_provider};
use super::postprocess;
use super::types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};
use crate::modules::voice::{is_hallucinated, HallucinationMode};

const LOG_PREFIX: &str = "[voice]";

/// Decides whether a raw transcript is a hallucination rather than speech.
///
/// A plain `fn` so the transcription pipelines take it as a parameter and
/// tests can substitute a fixed verdict for the TinyVoice module.
pub(crate) type HallucinationScreen =
    for<'a> fn(&'a Config, &'a str) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// The production screen: ask TinyVoice.
///
/// Falls OPEN when the module cannot be reached: passing a stock phrase
/// through costs the user one bad transcription they can see and redo,
/// whereas defaulting to "hallucinated" would silently delete real speech.
/// Only one of those is recoverable.
fn screen_with_tinyvoice<'a>(
    config: &'a Config,
    text: &'a str,
) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
    Box::pin(async move {
        match is_hallucinated(config, text, HallucinationMode::Conversation).await {
            Ok(verdict) => verdict,
            Err(error) => {
                warn!(
                    "{LOG_PREFIX} hallucination filter unavailable ({error}); passing text through"
                );
                false
            }
        }
    })
}

/// Check availability of the STT engine and the TTS binary/model without
/// executing them.
pub async fn voice_status(config: &Config) -> Result<Outcome<VoiceStatus>, String> {
    debug!("{LOG_PREFIX} checking voice status");

    let runtime = crate::inference::local_runtime_config(config);
    let piper_bin = tinyinference_local::service::paths::resolve_piper_binary_with_config(&runtime);
    let tts_voice = tinyinference_local::service::paths::resolve_tts_voice_path(&runtime).ok();

    // STT is hosted now, so "available" means the configured engine actually
    // resolves to a provider: the backend proxy always does, a third-party slug
    // only when its `voice_providers` entry exists. Constructing the provider is
    // the same check the transcribe path performs, and it makes no network call
    // — so this reports the real failure a user would hit, not a guess.
    let stt_engine = effective_stt_provider(config);
    let stt_error = match create_stt_provider(&stt_engine, "", config).await {
        Err(error) => Some(error.to_string()),
        Ok(_) => {
            let slug = stt_engine
                .split_once(':')
                .map_or(stt_engine.as_str(), |(slug, _)| slug);
            if matches!(slug.trim(), "cloud" | "openhuman" | "backend") {
                None
            } else {
                match crate::inference::provider::factory::lookup_key_for_slug(slug.trim(), config)
                    .await
                {
                    Ok(key) if !key.trim().is_empty() => None,
                    Ok(_) => Some(format!(
                        "voice provider '{slug}' has no API credential configured"
                    )),
                    Err(error) => Some(error.to_string()),
                }
            }
        }
    };
    let stt_available = stt_error.is_none();
    let tts_available = piper_bin.is_some() && tts_voice.is_some();

    debug!(
        "{LOG_PREFIX} stt_available={stt_available} stt_engine={stt_engine} \
         tts_available={tts_available} piper_bin={} tts_voice={} stt_error={:?}",
        safe_basename_path(&piper_bin),
        safe_basename_str(&tts_voice),
        stt_error,
    );

    // The provider synthesis actually uses: the top-level `tts_provider`
    // override wins over the legacy `local_ai.tts_provider`.
    let tts_provider = effective_tts_provider(config);

    let status = VoiceStatus {
        stt_available,
        tts_available,
        stt_model_id: model_ids::effective_stt_model_id(config),
        tts_voice_id: model_ids::effective_tts_voice_id(config),
        piper_binary: piper_bin.map(|p| p.display().to_string()),
        tts_voice_path: tts_voice,
        llm_cleanup_enabled: config.local_ai.voice_llm_cleanup_enabled,
        stt_engine,
        stt_error,
        tts_provider,
    };

    Ok(Outcome::single_log(status, "voice status checked"))
}

/// Transcribe audio from a file path through the configured STT engine.
///
/// If `context` is provided, the raw transcription is post-processed through
/// a local LLM to fix grammar and disambiguate words using conversation history.
pub async fn voice_transcribe(
    config: &Config,
    audio_path: &str,
    context: Option<&str>,
    skip_cleanup: bool,
) -> Result<Outcome<VoiceSpeechResult>, String> {
    transcribe_path_with(
        config,
        audio_path,
        context,
        skip_cleanup,
        screen_with_tinyvoice,
    )
    .await
}

/// [`voice_transcribe`] with the hallucination screen as a parameter.
pub(crate) async fn transcribe_path_with(
    config: &Config,
    audio_path: &str,
    context: Option<&str>,
    skip_cleanup: bool,
    screen: HallucinationScreen,
) -> Result<Outcome<VoiceSpeechResult>, String> {
    let started = Instant::now();
    debug!("{LOG_PREFIX} transcribing audio_path={audio_path}");

    let service = local_ai::global(config);
    let transcribe_started = Instant::now();
    // Context is forwarded as a vocabulary-bias hint where the engine has one.
    let output = local_ai::service::transcribe_with_prompt(
        service.as_ref(),
        config,
        audio_path.trim(),
        context,
    )
    .await
    .map_err(|e| e.to_string())?;
    debug!(
        "{LOG_PREFIX} transcription completed, text length={}, stt_elapsed_ms={}",
        output.text.len(),
        transcribe_started.elapsed().as_millis()
    );

    Ok(finish_transcription(
        config,
        output.text,
        output.model_id,
        context,
        skip_cleanup,
        screen,
        started,
    )
    .await)
}

/// Transcribe audio from raw bytes.
///
/// If `context` is provided, the raw transcription is post-processed through
/// a local LLM.
pub async fn voice_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
    context: Option<&str>,
    skip_cleanup: bool,
) -> Result<Outcome<VoiceSpeechResult>, String> {
    transcribe_bytes_with(
        config,
        audio_bytes,
        extension,
        context,
        skip_cleanup,
        screen_with_tinyvoice,
    )
    .await
}

/// [`voice_transcribe_bytes`] with the hallucination screen as a parameter.
pub(crate) async fn transcribe_bytes_with(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
    context: Option<&str>,
    skip_cleanup: bool,
    screen: HallucinationScreen,
) -> Result<Outcome<VoiceSpeechResult>, String> {
    let started = Instant::now();
    let ext = normalize_extension(extension)?;
    debug!(
        "{LOG_PREFIX} transcribe_bytes size={} ext={ext}",
        audio_bytes.len()
    );

    let service = local_ai::global(config);
    let transcribe_started = Instant::now();
    // The hosted engines take no vocabulary-bias prompt (see
    // `transcribe_with_prompt`), so `context` only feeds the LLM cleanup.
    let output =
        local_ai::service::transcribe_bytes(service.as_ref(), config, audio_bytes, Some(&ext))
            .await?;
    debug!(
        "{LOG_PREFIX} transcribe_bytes completed, text length={}, stt_elapsed_ms={}",
        output.text.len(),
        transcribe_started.elapsed().as_millis()
    );

    Ok(finish_transcription(
        config,
        output.text,
        output.model_id,
        context,
        skip_cleanup,
        screen,
        started,
    )
    .await)
}

/// The shared tail of both transcription entry points: drop hallucinated
/// output, then run the optional LLM cleanup.
async fn finish_transcription(
    config: &Config,
    raw_text: String,
    model_id: String,
    context: Option<&str>,
    skip_cleanup: bool,
    screen: HallucinationScreen,
    started: Instant,
) -> Outcome<VoiceSpeechResult> {
    // Filter hallucinated output before spending time on LLM cleanup.
    if screen(config, &raw_text).await {
        debug!("{LOG_PREFIX} hallucination detected, returning empty result");
        return Outcome::single_log(
            VoiceSpeechResult {
                text: String::new(),
                raw_text,
                model_id,
            },
            "voice transcription filtered (hallucination)",
        );
    }

    let cleanup_started = Instant::now();
    let text = if skip_cleanup {
        raw_text.clone()
    } else {
        postprocess::cleanup_transcription(config, &raw_text, context).await
    };
    debug!(
        "{LOG_PREFIX} transcription pipeline complete (cleanup_elapsed_ms={}, total_elapsed_ms={})",
        cleanup_started.elapsed().as_millis(),
        started.elapsed().as_millis()
    );

    Outcome::single_log(
        VoiceSpeechResult {
            text,
            raw_text,
            model_id,
        },
        "voice transcription completed",
    )
}

/// Synthesize speech from text using piper.
pub async fn voice_tts(
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<Outcome<VoiceTtsResult>, String> {
    debug!(
        "{LOG_PREFIX} tts text_length={} output_path={:?}",
        text.len(),
        output_path
    );

    let service = local_ai::global(config);
    let output = local_ai::service::tts(service.as_ref(), config, text.trim(), output_path)
        .await
        .map_err(|e| e.to_string())?;

    debug!("{LOG_PREFIX} tts completed, output={}", output.output_path);

    Ok(Outcome::single_log(
        VoiceTtsResult::from(output),
        "voice tts completed",
    ))
}

/// Normalize an optional audio file extension. Returns a clean lowercase
/// alphanumeric extension string, defaulting to "webm".
pub(crate) fn normalize_extension(ext: Option<String>) -> Result<String, String> {
    let normalized = ext
        .unwrap_or_else(|| "webm".to_string())
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();

    if normalized.is_empty() {
        return Err("audio extension must not be empty".to_string());
    }
    if !normalized.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "invalid audio extension '{normalized}': must be alphanumeric"
        ));
    }

    Ok(normalized)
}

/// Extract the file name from an `Option<PathBuf>`, returning `"<none>"` if absent.
fn safe_basename_path(p: &Option<std::path::PathBuf>) -> String {
    p.as_ref()
        .and_then(|pb| pb.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("<none>")
        .to_string()
}

/// Extract the file name from an `Option<String>` path, returning `"<none>"` if absent.
fn safe_basename_str(p: &Option<String>) -> String {
    p.as_ref()
        .and_then(|s| std::path::Path::new(s).file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("<none>")
        .to_string()
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
