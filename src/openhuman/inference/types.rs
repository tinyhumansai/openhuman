//! Serializable DTOs for local AI status and RPC responses.

use crate::openhuman::config::Config;
use serde::{Deserialize, Serialize};

use super::local::provider::provider_from_config;
use super::model_ids;
use super::presets;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiStatus {
    pub state: String,
    pub model_id: String,
    pub chat_model_id: String,
    pub vision_model_id: String,
    pub embedding_model_id: String,
    pub stt_model_id: String,
    pub tts_voice_id: String,
    pub quantization: String,
    pub vision_state: String,
    pub vision_mode: String,
    pub embedding_state: String,
    pub stt_state: String,
    pub tts_state: String,
    pub provider: String,
    pub download_progress: Option<f32>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub download_speed_bps: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub warning: Option<String>,
    /// Extended error text (e.g. stderr from install script) for UI display.
    pub error_detail: Option<String>,
    /// Category of failure: "install", "download", "server", or None.
    pub error_category: Option<String>,
    pub model_path: Option<String>,
    pub active_backend: String,
    pub backend_reason: Option<String>,
    pub last_latency_ms: Option<u64>,
    pub prompt_toks_per_sec: Option<f32>,
    pub gen_toks_per_sec: Option<f32>,
}

impl LocalAiStatus {
    pub(crate) fn disabled(config: &Config) -> Self {
        let vision_mode = presets::vision_mode_for_config(&config.local_ai);
        let provider = provider_from_config(config);
        Self {
            state: "disabled".to_string(),
            model_id: model_ids::effective_chat_model_id(config),
            chat_model_id: model_ids::effective_chat_model_id(config),
            vision_model_id: model_ids::effective_vision_model_id(config),
            embedding_model_id: model_ids::effective_embedding_model_id(config),
            stt_model_id: model_ids::effective_stt_model_id(config),
            tts_voice_id: model_ids::effective_tts_voice_id(config),
            quantization: model_ids::effective_quantization(config),
            vision_state: "disabled".to_string(),
            vision_mode: format!("{vision_mode:?}").to_ascii_lowercase(),
            embedding_state: "disabled".to_string(),
            stt_state: "disabled".to_string(),
            tts_state: "disabled".to_string(),
            provider: provider.as_str().to_string(),
            download_progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            download_speed_bps: None,
            eta_seconds: None,
            warning: None,
            error_detail: None,
            error_category: None,
            model_path: None,
            active_backend: provider.as_str().to_string(),
            backend_reason: None,
            last_latency_ms: None,
            prompt_toks_per_sec: None,
            gen_toks_per_sec: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiAssetStatus {
    pub state: String,
    pub id: String,
    pub provider: String,
    pub path: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiAssetsStatus {
    pub chat: LocalAiAssetStatus,
    pub vision: LocalAiAssetStatus,
    pub embedding: LocalAiAssetStatus,
    pub tts: LocalAiAssetStatus,
    pub quantization: String,
    /// True when the configured Ollama endpoint is reachable enough for model
    /// checks. When false, the frontend should render external-runtime
    /// guidance rather than app-managed install/start affordances.
    pub ollama_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiDownloadProgressItem {
    pub id: String,
    pub provider: String,
    pub state: String,
    pub progress: Option<f32>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub speed_bps: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub warning: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiDownloadsProgress {
    pub state: String,
    pub warning: Option<String>,
    pub progress: Option<f32>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub speed_bps: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub chat: LocalAiDownloadProgressItem,
    pub vision: LocalAiDownloadProgressItem,
    pub embedding: LocalAiDownloadProgressItem,
    pub tts: LocalAiDownloadProgressItem,
    /// Mirrors `LocalAiAssetsStatus::ollama_available` so a single
    /// `local_ai.downloads_progress` poll can render the right UI state.
    pub ollama_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiEmbeddingResult {
    pub model_id: String,
    pub dimensions: usize,
    pub vectors: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiSpeechResult {
    pub text: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiTtsResult {
    pub output_path: String,
    pub voice_id: String,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
