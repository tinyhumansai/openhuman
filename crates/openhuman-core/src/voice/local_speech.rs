//! OpenHuman configuration adapter for local Piper text-to-speech.

use crate::config::Config;
use crate::core::Outcome;
#[cfg(test)]
use tinyinference_voice::piper::synthetic_viseme_timeline;
use tinyinference_voice::reply::ReplySpeech as ReplySpeechResult;

/// Default Piper voice id.
pub const DEFAULT_PIPER_VOICE: &str = "en_US-lessac-medium";

/// Caller-tunable local synthesis options.
#[derive(Debug, Default, Clone)]
pub struct PiperOptions {
    /// Optional voice identifier reported to the caller.
    pub voice: Option<String>,
}

/// Resolve OpenHuman's local runtime paths and synthesize speech with TinyInference.
pub async fn synthesize_piper(
    config: &Config,
    text: &str,
    options: &PiperOptions,
) -> Result<Outcome<ReplySpeechResult>, String> {
    if text.trim().is_empty() {
        return Err("text is required".to_string());
    }
    let runtime = crate::inference::local_runtime_config(config);
    let binary = tinyinference_local::service::paths::resolve_piper_binary_with_config(&runtime)
        .ok_or_else(|| "piper binary not found. Set PIPER_BIN or install piper.".to_string())?;
    let model = tinyinference_local::service::paths::resolve_tts_voice_path(&runtime)?;
    let voice = options
        .voice
        .as_deref()
        .map(str::trim)
        .filter(|voice| !voice.is_empty())
        .unwrap_or(DEFAULT_PIPER_VOICE);
    log::debug!("[voice-tts] synthesizing voice={voice}");
    let speech = tinyinference_voice::piper::synthesize(&binary, model.as_ref(), text).await?;
    Ok(Outcome::single_log(
        ReplySpeechResult {
            audio_base64: speech.audio_base64,
            audio_mime: speech.audio_mime,
            visemes: speech.visemes,
            alignment: None,
        },
        "local piper TTS completed",
    ))
}

#[cfg(test)]
#[path = "local_speech_tests.rs"]
mod tests;
