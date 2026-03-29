//! Resolved model / voice IDs from [`crate::openhuman::config::Config`].

use crate::openhuman::config::Config;

pub(crate) const DEFAULT_OLLAMA_MODEL: &str = "gemma3:4b-it-qat";
pub(crate) const DEFAULT_OLLAMA_VISION_MODEL: &str = "gemma3:4b-it-qat";
pub(crate) const DEFAULT_OLLAMA_EMBED_MODEL: &str = "nomic-embed-text:latest";

pub(crate) fn effective_chat_model_id(config: &Config) -> String {
    let raw = if !config.local_ai.chat_model_id.trim().is_empty() {
        config.local_ai.chat_model_id.trim()
    } else {
        config.local_ai.model_id.trim()
    };
    if raw.is_empty() {
        return DEFAULT_OLLAMA_MODEL.to_string();
    }
    let lower = raw.to_ascii_lowercase();
    if lower.ends_with(".gguf")
        || lower.contains("huggingface.co/")
        || lower == "qwen3-1.7b"
        || lower == "qwen2.5-1.5b-instruct"
    {
        return DEFAULT_OLLAMA_MODEL.to_string();
    }
    raw.to_string()
}

pub(crate) fn effective_vision_model_id(config: &Config) -> String {
    let raw = config.local_ai.vision_model_id.trim();
    if raw.is_empty() {
        return DEFAULT_OLLAMA_VISION_MODEL.to_string();
    }
    let lower = raw.to_ascii_lowercase();
    if lower == "moondream:1.8b" || lower == "moondream" {
        return DEFAULT_OLLAMA_VISION_MODEL.to_string();
    }
    raw.to_string()
}

pub(crate) fn effective_embedding_model_id(config: &Config) -> String {
    let raw = config.local_ai.embedding_model_id.trim();
    if raw.is_empty() {
        return DEFAULT_OLLAMA_EMBED_MODEL.to_string();
    }
    raw.to_string()
}

pub(crate) fn effective_stt_model_id(config: &Config) -> String {
    let raw = config.local_ai.stt_model_id.trim();
    if raw.is_empty() {
        "ggml-tiny-q5_1.bin".to_string()
    } else {
        raw.to_string()
    }
}

pub(crate) fn effective_tts_voice_id(config: &Config) -> String {
    let raw = config.local_ai.tts_voice_id.trim();
    if raw.is_empty() {
        "en_US-lessac-medium".to_string()
    } else {
        raw.to_string()
    }
}

pub(crate) fn effective_quantization(config: &Config) -> String {
    let raw = config.local_ai.quantization.trim();
    if raw.is_empty() {
        "q4".to_string()
    } else {
        raw.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn chat_model_falls_back_for_empty_and_unsupported_ids() {
        let mut config = test_config();

        config.local_ai.chat_model_id = String::new();
        config.local_ai.model_id = String::new();
        assert_eq!(effective_chat_model_id(&config), DEFAULT_OLLAMA_MODEL);

        config.local_ai.chat_model_id = "custom.gguf".to_string();
        assert_eq!(effective_chat_model_id(&config), DEFAULT_OLLAMA_MODEL);

        config.local_ai.chat_model_id = "qwen3-1.7b".to_string();
        assert_eq!(effective_chat_model_id(&config), DEFAULT_OLLAMA_MODEL);
    }

    #[test]
    fn chat_model_prefers_explicit_supported_chat_model() {
        let mut config = test_config();
        config.local_ai.model_id = "fallback:model".to_string();
        config.local_ai.chat_model_id = "gemma3:4b-it-qat".to_string();
        assert_eq!(effective_chat_model_id(&config), "gemma3:4b-it-qat");
    }

    #[test]
    fn vision_model_normalizes_legacy_moondream_values() {
        let mut config = test_config();
        config.local_ai.vision_model_id = "moondream".to_string();
        assert_eq!(
            effective_vision_model_id(&config),
            DEFAULT_OLLAMA_VISION_MODEL
        );
        config.local_ai.vision_model_id = "moondream:1.8b".to_string();
        assert_eq!(
            effective_vision_model_id(&config),
            DEFAULT_OLLAMA_VISION_MODEL
        );
    }

    #[test]
    fn stt_tts_and_quantization_defaults_are_applied() {
        let mut config = test_config();
        config.local_ai.stt_model_id.clear();
        config.local_ai.tts_voice_id.clear();
        config.local_ai.quantization = "Q5_K_M".to_string();

        assert_eq!(effective_stt_model_id(&config), "ggml-tiny-q5_1.bin");
        assert_eq!(effective_tts_voice_id(&config), "en_US-lessac-medium");
        assert_eq!(effective_quantization(&config), "q5_k_m");
    }
}
