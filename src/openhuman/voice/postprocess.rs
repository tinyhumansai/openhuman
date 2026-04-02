//! LLM-based post-processing for voice transcription.
//!
//! Passes raw whisper output through a local LLM (Ollama) to clean up
//! grammar, punctuation, and filler words. Optionally uses conversation
//! context to disambiguate unclear words (names, technical terms).

use log::{debug, warn};

use crate::openhuman::config::Config;
use crate::openhuman::local_ai;

const LOG_PREFIX: &str = "[voice_postprocess]";

const CLEANUP_SYSTEM_PROMPT: &str = "\
You clean up voice transcription text. Fix grammar, punctuation, and \
remove filler words (um, uh, like). Keep the original meaning intact. \
If conversation context is provided, use it to disambiguate unclear \
words (names, technical terms). Return ONLY the corrected text, \
nothing else.";

/// Clean up raw transcription text using a local LLM.
///
/// Returns the cleaned text on success, or the original raw text if the
/// LLM is unavailable or cleanup fails (graceful degradation).
pub async fn cleanup_transcription(
    config: &Config,
    raw_text: &str,
    conversation_context: Option<&str>,
) -> String {
    if raw_text.trim().is_empty() {
        return raw_text.to_string();
    }

    if !config.local_ai.voice_llm_cleanup_enabled {
        debug!("{LOG_PREFIX} LLM cleanup disabled in config");
        return raw_text.to_string();
    }

    debug!(
        "{LOG_PREFIX} cleaning up transcription ({} chars, context={})",
        raw_text.len(),
        conversation_context.is_some()
    );

    let prompt = match conversation_context {
        Some(ctx) if !ctx.trim().is_empty() => {
            format!(
                "Conversation context:\n{ctx}\n\n\
                 Transcribed text to clean up:\n{raw_text}"
            )
        }
        _ => raw_text.to_string(),
    };

    let service = local_ai::global(config);

    let result: Result<String, String> = service
        .inference(config, CLEANUP_SYSTEM_PROMPT, &prompt, Some(512), true)
        .await;

    match result {
        Ok(ref cleaned_ref) => {
            let cleaned = cleaned_ref.trim().to_string();
            if cleaned.is_empty() {
                warn!("{LOG_PREFIX} LLM returned empty cleanup, using raw text");
                raw_text.to_string()
            } else {
                debug!(
                    "{LOG_PREFIX} cleanup complete: {} chars -> {} chars",
                    raw_text.len(),
                    cleaned.len()
                );
                cleaned
            }
        }
        Err(e) => {
            warn!("{LOG_PREFIX} LLM cleanup failed, using raw text: {e}");
            raw_text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_unchanged() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = Config::default();
        let result = rt.block_on(cleanup_transcription(&config, "", None));
        assert_eq!(result, "");
    }

    #[test]
    fn whitespace_only_returns_unchanged() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = Config::default();
        let result = rt.block_on(cleanup_transcription(&config, "   ", None));
        assert_eq!(result, "   ");
    }

    #[test]
    fn disabled_cleanup_returns_raw_text() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut config = Config::default();
        config.local_ai.voice_llm_cleanup_enabled = false;
        let result = rt.block_on(cleanup_transcription(&config, "um hello uh world", None));
        assert_eq!(result, "um hello uh world");
    }
}
