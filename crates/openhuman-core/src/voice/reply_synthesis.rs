//! Spoken-reply synthesis for chat surfaces (web chat and channel replies).
//!
//! Routes through the TTS factory so the user's configured `tts_provider` is
//! honoured, the same way `voice.reply_synthesize` does.

use tinyinference_voice::reply::ReplySpeech;

use crate::config::Config;
use crate::core::Outcome;

const LOG_PREFIX: &str = "[voice_reply]";

/// Synthesize `text` with the effective TTS provider. An empty `voice_id`
/// lets the provider pick its default (Piper falls back to its stock voice).
#[cfg(feature = "voice")]
pub async fn synthesize_reply(
    config: &Config,
    text: &str,
    voice_id: Option<&str>,
) -> Result<Outcome<ReplySpeech>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("text is required".to_string());
    }
    let provider_name = super::effective_tts_provider(config);
    let voice = voice_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if provider_name == "piper" {
                super::DEFAULT_PIPER_VOICE.to_owned()
            } else {
                String::new()
            }
        });
    log::debug!(
        "{LOG_PREFIX} synthesize provider={provider_name} chars={}",
        trimmed.chars().count()
    );
    let provider =
        super::create_tts_provider(&provider_name, &voice, config).map_err(|e| e.to_string())?;
    let effective = (!voice.is_empty()).then_some(voice.as_str());
    provider.synthesize(config, trimmed, effective).await
}

/// The `voice` feature is off: there is no TTS factory to route through.
#[cfg(not(feature = "voice"))]
pub async fn synthesize_reply(
    _config: &Config,
    _text: &str,
    _voice_id: Option<&str>,
) -> Result<Outcome<ReplySpeech>, String> {
    Err("voice is not compiled into this build".to_string())
}
