//! Resolved model / voice IDs from [`crate::openhuman::config::Config`].
//!
//! All `effective_*` functions enforce the MVP model allowlist: if a resolved
//! model ID is not in the allowlist the function silently falls back to the
//! default MVP model and logs a warning. This prevents config-file edits from
//! bypassing the MVP tier restriction.

use crate::openhuman::config::Config;

pub(crate) const DEFAULT_OLLAMA_MODEL: &str = "gemma3:4b-it-qat";
pub(crate) const DEFAULT_OLLAMA_VISION_MODEL: &str = "gemma3:4b-it-qat";
pub(crate) const DEFAULT_LOW_VISION_MODEL: &str = "moondream:1.8b-v2-q4_K_S";
pub(crate) const DEFAULT_OLLAMA_EMBED_MODEL: &str = "nomic-embed-text:latest";

/// Chat models allowed in the current MVP build (2–4 GB tier only).
/// Any resolved chat model ID not listed here is redirected to `MVP_DEFAULT_CHAT_MODEL`.
const MVP_ALLOWED_CHAT_MODELS: &[&str] = &["gemma3:1b-it-qat"];
const MVP_DEFAULT_CHAT_MODEL: &str = "gemma3:1b-it-qat";

/// Vision models allowed in MVP — only disabled (empty string) since the
/// 2–4 GB tier has no vision model.
const MVP_ALLOWED_VISION_MODELS: &[&str] = &[""];

/// Embedding models allowed in MVP (2–4 GB tier uses all-minilm).
const MVP_ALLOWED_EMBEDDING_MODELS: &[&str] = &["all-minilm:latest"];

fn enforce_mvp_chat_allowlist(resolved: &str) -> String {
    let lower = resolved.to_ascii_lowercase();
    for allowed in MVP_ALLOWED_CHAT_MODELS {
        if lower == allowed.to_ascii_lowercase() {
            return resolved.to_string();
        }
    }
    tracing::warn!(
        resolved,
        fallback = MVP_DEFAULT_CHAT_MODEL,
        "[local_ai] chat model not in MVP allowlist, redirecting to default"
    );
    MVP_DEFAULT_CHAT_MODEL.to_string()
}

fn enforce_mvp_vision_allowlist(resolved: &str) -> String {
    let lower = resolved.to_ascii_lowercase();
    for allowed in MVP_ALLOWED_VISION_MODELS {
        if lower == allowed.to_ascii_lowercase() {
            return resolved.to_string();
        }
    }
    tracing::warn!(
        resolved,
        "[local_ai] vision model not in MVP allowlist, disabling vision"
    );
    String::new()
}

fn enforce_mvp_embedding_allowlist(resolved: &str) -> String {
    let lower = resolved.to_ascii_lowercase();
    for allowed in MVP_ALLOWED_EMBEDDING_MODELS {
        if lower == allowed.to_ascii_lowercase() {
            return resolved.to_string();
        }
    }
    tracing::warn!(
        resolved,
        fallback = MVP_ALLOWED_EMBEDDING_MODELS[0],
        "[local_ai] embedding model not in MVP allowlist, redirecting to default"
    );
    MVP_ALLOWED_EMBEDDING_MODELS[0].to_string()
}

pub(crate) fn effective_chat_model_id(config: &Config) -> String {
    let raw = if !config.local_ai.chat_model_id.trim().is_empty() {
        config.local_ai.chat_model_id.trim()
    } else {
        config.local_ai.model_id.trim()
    };
    if raw.is_empty() {
        return enforce_mvp_chat_allowlist(DEFAULT_OLLAMA_MODEL);
    }
    let lower = raw.to_ascii_lowercase();
    if lower.ends_with(".gguf")
        || lower.contains("huggingface.co/")
        || lower == "qwen3-1.7b"
        || lower == "qwen2.5-1.5b-instruct"
    {
        return enforce_mvp_chat_allowlist(DEFAULT_OLLAMA_MODEL);
    }
    enforce_mvp_chat_allowlist(raw)
}

pub(crate) fn effective_vision_model_id(config: &Config) -> String {
    let raw = config.local_ai.vision_model_id.trim();
    if raw.is_empty() {
        return String::new();
    }
    let lower = raw.to_ascii_lowercase();
    let resolved = if lower == "moondream:1.8b" || lower == "moondream" {
        DEFAULT_LOW_VISION_MODEL
    } else {
        raw
    };
    enforce_mvp_vision_allowlist(resolved)
}

pub(crate) fn effective_embedding_model_id(config: &Config) -> String {
    let raw = config.local_ai.embedding_model_id.trim();
    if raw.is_empty() {
        return enforce_mvp_embedding_allowlist(DEFAULT_OLLAMA_EMBED_MODEL);
    }
    enforce_mvp_embedding_allowlist(raw)
}

pub(crate) fn effective_stt_model_id(config: &Config) -> String {
    let raw = config.local_ai.stt_model_id.trim();
    if raw.is_empty() {
        "ggml-base-q5_1.bin".to_string()
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
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        config.local_ai.chat_model_id = "custom.gguf".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        config.local_ai.chat_model_id = "qwen3-1.7b".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);
    }

    #[test]
    fn chat_model_allows_mvp_model() {
        let mut config = test_config();
        config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
        assert_eq!(effective_chat_model_id(&config), "gemma3:1b-it-qat");
    }

    #[test]
    fn chat_model_rejects_non_mvp_models() {
        let mut config = test_config();
        // All models outside the single MVP-allowed model are rejected.
        config.local_ai.chat_model_id = "gemma3:4b-it-qat".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        config.local_ai.chat_model_id = "gemma3:270m-it-qat".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        config.local_ai.chat_model_id = "gemma4:e4b".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);
    }

    #[test]
    fn vision_model_normalizes_legacy_moondream_values() {
        let mut config = test_config();
        config.local_ai.vision_model_id = String::new();
        assert_eq!(effective_vision_model_id(&config), "");

        // Moondream is not in the MVP vision allowlist (only "" is allowed),
        // so it gets redirected to "" (vision disabled).
        config.local_ai.vision_model_id = "moondream".to_string();
        assert_eq!(effective_vision_model_id(&config), "");
        config.local_ai.vision_model_id = "moondream:1.8b".to_string();
        assert_eq!(effective_vision_model_id(&config), "");
    }

    #[test]
    fn stt_tts_and_quantization_defaults_are_applied() {
        let mut config = test_config();
        config.local_ai.stt_model_id.clear();
        config.local_ai.tts_voice_id.clear();
        config.local_ai.quantization = "Q5_K_M".to_string();

        assert_eq!(effective_stt_model_id(&config), "ggml-base-q5_1.bin");
        assert_eq!(effective_tts_voice_id(&config), "en_US-lessac-medium");
        assert_eq!(effective_quantization(&config), "q5_k_m");
    }
}
