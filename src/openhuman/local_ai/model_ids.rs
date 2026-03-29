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
