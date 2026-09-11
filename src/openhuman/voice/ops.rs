//! Voice domain business logic — hosted STT and local piper TTS.
//!
//! Each public function follows the `RpcOutcome<T>` pattern used by other
//! domain modules (billing, health, etc.).

use chrono::Utc;
use log::{debug, warn};
use std::time::Instant;

use crate::openhuman::config::Config;
use crate::openhuman::inference::local as local_ai;
use crate::openhuman::inference::local::model_ids;
use crate::openhuman::inference::local::paths::{resolve_piper_binary, resolve_tts_voice_path};
use crate::rpc::RpcOutcome;

use super::factory::{create_stt_provider, effective_stt_provider};
use super::postprocess;
use super::types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};
use crate::openhuman::modules::voice::{is_hallucinated, HallucinationMode};

const LOG_PREFIX: &str = "[voice]";

/// Check availability of the STT engine and the TTS binary/model without
/// executing them.
pub async fn voice_status(config: &Config) -> Result<RpcOutcome<VoiceStatus>, String> {
    debug!("{LOG_PREFIX} checking voice status");

    let piper_bin = resolve_piper_binary();
    let tts_voice = resolve_tts_voice_path(config).ok();

    // STT is hosted now, so "available" means the configured engine actually
    // resolves to a provider: the backend proxy always does, a third-party slug
    // only when its `voice_providers` entry exists. Constructing the provider is
    // the same check the transcribe path performs, and it makes no network call
    // — so this reports the real failure a user would hit, not a guess.
    let stt_engine = effective_stt_provider(config);
    let stt_error = create_stt_provider(&stt_engine, "", config)
        .and_then(|_| {
            let slug = stt_engine
                .split_once(':')
                .map_or(stt_engine.as_str(), |(slug, _)| slug);
            if matches!(slug.trim(), "cloud" | "openhuman" | "backend") {
                return Ok(());
            }
            let key = crate::openhuman::inference::provider::factory::lookup_key_for_slug(
                slug.trim(),
                config,
            )?;
            if key.trim().is_empty() {
                anyhow::bail!("voice provider '{slug}' has no API credential configured")
            }
            Ok(())
        })
        .err()
        .map(|e| e.to_string());
    let stt_available = stt_error.is_none();
    let tts_available = piper_bin.is_some() && tts_voice.is_some();

    debug!(
        "{LOG_PREFIX} stt_available={stt_available} stt_engine={stt_engine} \
         tts_available={tts_available} piper_bin={} tts_voice={} stt_error={:?}",
        safe_basename_path(&piper_bin),
        safe_basename_str(&tts_voice),
        stt_error,
    );

    let tts_provider = if config.local_ai.tts_provider.trim().is_empty() {
        "cloud".to_string()
    } else {
        config.local_ai.tts_provider.clone()
    };

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

    Ok(RpcOutcome::single_log(status, "voice status checked"))
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
) -> Result<RpcOutcome<VoiceSpeechResult>, String> {
    let started = Instant::now();
    debug!("{LOG_PREFIX} transcribing audio_path={audio_path}");

    let service = local_ai::global(config);
    let transcribe_started = Instant::now();
    // Context is forwarded as a vocabulary-bias hint where the engine has one.
    let output = service
        .transcribe_with_prompt(config, audio_path.trim(), context)
        .await
        .map_err(|e| e.to_string())?;
    let transcribe_elapsed = transcribe_started.elapsed();

    let raw_text = output.text.clone();
    debug!(
        "{LOG_PREFIX} transcription completed, text length={}, stt_elapsed_ms={}",
        raw_text.len(),
        transcribe_elapsed.as_millis()
    );

    let cleanup_started = Instant::now();
    let text = if skip_cleanup {
        raw_text.clone()
    } else {
        postprocess::cleanup_transcription(config, &raw_text, context).await
    };
    let cleanup_elapsed = cleanup_started.elapsed();
    debug!(
        "{LOG_PREFIX} voice_transcribe complete (cleanup_elapsed_ms={}, total_elapsed_ms={})",
        cleanup_elapsed.as_millis(),
        started.elapsed().as_millis()
    );

    Ok(RpcOutcome::single_log(
        VoiceSpeechResult {
            text,
            raw_text,
            model_id: output.model_id,
        },
        "voice transcription completed",
    ))
}

/// Transcribe audio from raw bytes. Writes to a temp file, transcribes, cleans up.
///
/// If `context` is provided, the raw transcription is post-processed through
/// a local LLM.
pub async fn voice_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
    context: Option<&str>,
    skip_cleanup: bool,
) -> Result<RpcOutcome<VoiceSpeechResult>, String> {
    let started = Instant::now();
    let ext = normalize_extension(extension)?;
    debug!(
        "{LOG_PREFIX} transcribe_bytes size={} ext={ext}",
        audio_bytes.len()
    );

    let service = local_ai::global(config);

    let voice_dir = std::env::temp_dir().join("openhuman_voice_input");
    tokio::fs::create_dir_all(&voice_dir)
        .await
        .map_err(|e| format!("failed to create voice input directory: {e}"))?;

    let filename = format!(
        "voice-{}-{}.{}",
        Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4(),
        ext
    );
    let file_path = voice_dir.join(filename);
    let write_started = Instant::now();
    tokio::fs::write(&file_path, audio_bytes)
        .await
        .map_err(|e| format!("failed to write audio file: {e}"))?;
    let write_elapsed = write_started.elapsed();

    let transcribe_started = Instant::now();
    // Context is forwarded as a vocabulary-bias hint where the engine has one.
    let output = service
        .transcribe_with_prompt(config, file_path.to_string_lossy().as_ref(), context)
        .await;
    let transcribe_elapsed = transcribe_started.elapsed();
    if let Err(e) = tokio::fs::remove_file(&file_path).await {
        warn!(
            "{LOG_PREFIX} failed to clean up temp audio file {}: {e}",
            file_path.display()
        );
    }

    let output = output.map_err(|e| e.to_string())?;
    let raw_text = output.text.clone();

    debug!(
        "{LOG_PREFIX} transcribe_bytes completed, text length={}, write_elapsed_ms={}, stt_elapsed_ms={}",
        raw_text.len(),
        write_elapsed.as_millis(),
        transcribe_elapsed.as_millis()
    );

    // Filter hallucinated output before spending time on LLM cleanup.
    //
    // Falls OPEN when the module cannot be reached: passing a stock phrase
    // through costs the user one bad transcription they can see and redo,
    // whereas defaulting to "hallucinated" would silently delete real speech.
    // Only one of those is recoverable.
    let hallucinated = match is_hallucinated(config, &raw_text, HallucinationMode::Conversation)
        .await
    {
        Ok(verdict) => verdict,
        Err(error) => {
            warn!("{LOG_PREFIX} hallucination filter unavailable ({error}); passing text through");
            false
        }
    };
    if hallucinated {
        debug!("{LOG_PREFIX} transcribe_bytes: hallucination detected, returning empty result");
        return Ok(RpcOutcome::single_log(
            VoiceSpeechResult {
                text: String::new(),
                raw_text,
                model_id: output.model_id,
            },
            "voice transcription filtered (hallucination)",
        ));
    }

    let cleanup_started = Instant::now();
    let text = if skip_cleanup {
        raw_text.clone()
    } else {
        postprocess::cleanup_transcription(config, &raw_text, context).await
    };
    let cleanup_elapsed = cleanup_started.elapsed();
    debug!(
        "{LOG_PREFIX} transcribe_bytes pipeline complete (cleanup_elapsed_ms={}, total_elapsed_ms={})",
        cleanup_elapsed.as_millis(),
        started.elapsed().as_millis()
    );

    Ok(RpcOutcome::single_log(
        VoiceSpeechResult {
            text,
            raw_text,
            model_id: output.model_id,
        },
        "voice transcription completed",
    ))
}

/// Synthesize speech from text using piper.
pub async fn voice_tts(
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<RpcOutcome<VoiceTtsResult>, String> {
    debug!(
        "{LOG_PREFIX} tts text_length={} output_path={:?}",
        text.len(),
        output_path
    );

    let service = local_ai::global(config);
    let output = service
        .tts(config, text.trim(), output_path)
        .await
        .map_err(|e| e.to_string())?;

    debug!("{LOG_PREFIX} tts completed, output={}", output.output_path);

    Ok(RpcOutcome::single_log(
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
