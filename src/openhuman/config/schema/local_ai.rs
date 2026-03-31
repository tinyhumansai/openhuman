//! Local AI runtime configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalAiConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model_id")]
    pub model_id: String,
    #[serde(default = "default_chat_model_id")]
    pub chat_model_id: String,
    #[serde(default = "default_vision_model_id")]
    pub vision_model_id: String,
    #[serde(default = "default_embedding_model_id")]
    pub embedding_model_id: String,
    #[serde(default = "default_stt_model_id")]
    pub stt_model_id: String,
    #[serde(default = "default_stt_download_url")]
    pub stt_download_url: Option<String>,
    #[serde(default = "default_tts_voice_id")]
    pub tts_voice_id: String,
    #[serde(default = "default_tts_download_url")]
    pub tts_download_url: Option<String>,
    #[serde(default = "default_tts_config_download_url")]
    pub tts_config_download_url: Option<String>,
    #[serde(default = "default_quantization")]
    pub quantization: String,
    #[serde(default = "default_preload_vision_model")]
    pub preload_vision_model: bool,
    #[serde(default = "default_preload_embedding_model")]
    pub preload_embedding_model: bool,
    #[serde(default = "default_preload_stt_model")]
    pub preload_stt_model: bool,
    #[serde(default = "default_preload_tts_voice")]
    pub preload_tts_voice: bool,
    #[serde(default = "default_download_url")]
    pub download_url: Option<String>,
    #[serde(default)]
    pub checksum_sha256: Option<String>,
    #[serde(default = "default_artifact_name")]
    pub artifact_name: String,
    #[serde(default = "default_autosummary_debounce_ms")]
    pub autosummary_debounce_ms: u64,
    #[serde(default = "default_context_compaction_threshold_tokens")]
    pub context_compaction_threshold_tokens: usize,
    #[serde(default = "default_max_suggestions")]
    pub max_suggestions: usize,
    #[serde(default)]
    pub selected_tier: Option<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_provider() -> String {
    "ollama".to_string()
}

fn default_model_id() -> String {
    "gemma3:4b-it-qat".to_string()
}

fn default_chat_model_id() -> String {
    "gemma3:4b-it-qat".to_string()
}

fn default_vision_model_id() -> String {
    "gemma3:4b-it-qat".to_string()
}

fn default_embedding_model_id() -> String {
    "nomic-embed-text:latest".to_string()
}

fn default_stt_model_id() -> String {
    "ggml-tiny-q5_1.bin".to_string()
}

fn default_tts_voice_id() -> String {
    "en_US-lessac-medium".to_string()
}

fn default_stt_download_url() -> Option<String> {
    Some(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin?download=true"
            .to_string(),
    )
}

fn default_tts_download_url() -> Option<String> {
    Some(
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx?download=true"
            .to_string(),
    )
}

fn default_tts_config_download_url() -> Option<String> {
    Some(
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json?download=true"
            .to_string(),
    )
}

fn default_quantization() -> String {
    "q4".to_string()
}

fn default_preload_vision_model() -> bool {
    false
}

fn default_preload_embedding_model() -> bool {
    true
}

fn default_preload_stt_model() -> bool {
    false
}

fn default_preload_tts_voice() -> bool {
    false
}

fn default_download_url() -> Option<String> {
    None
}

fn default_artifact_name() -> String {
    "ollama-managed".to_string()
}

fn default_autosummary_debounce_ms() -> u64 {
    2500
}

fn default_context_compaction_threshold_tokens() -> usize {
    100_000
}

fn default_max_suggestions() -> usize {
    5
}

impl Default for LocalAiConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            provider: default_provider(),
            model_id: default_model_id(),
            chat_model_id: default_chat_model_id(),
            vision_model_id: default_vision_model_id(),
            embedding_model_id: default_embedding_model_id(),
            stt_model_id: default_stt_model_id(),
            stt_download_url: default_stt_download_url(),
            tts_voice_id: default_tts_voice_id(),
            tts_download_url: default_tts_download_url(),
            tts_config_download_url: default_tts_config_download_url(),
            quantization: default_quantization(),
            preload_vision_model: default_preload_vision_model(),
            preload_embedding_model: default_preload_embedding_model(),
            preload_stt_model: default_preload_stt_model(),
            preload_tts_voice: default_preload_tts_voice(),
            download_url: default_download_url(),
            checksum_sha256: None,
            artifact_name: default_artifact_name(),
            autosummary_debounce_ms: default_autosummary_debounce_ms(),
            context_compaction_threshold_tokens: default_context_compaction_threshold_tokens(),
            max_suggestions: default_max_suggestions(),
            selected_tier: None,
        }
    }
}
