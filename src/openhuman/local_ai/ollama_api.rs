//! Ollama HTTP JSON types and small helpers (private to this crate).

use serde::{Deserialize, Serialize};

pub(crate) const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Serialize)]
pub(crate) struct OllamaPullRequest {
    pub name: String,
    pub stream: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaPullEvent {
    #[allow(dead_code)]
    pub status: Option<String>,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaTagsResponse {
    #[serde(default)]
    pub models: Vec<OllamaModelTag>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaModelTag {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaGenerateRequest {
    pub model: String,
    pub prompt: String,
    pub system: Option<String>,
    pub images: Option<Vec<String>>,
    pub stream: bool,
    pub options: Option<OllamaGenerateOptions>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaGenerateOptions {
    pub temperature: Option<f32>,
    pub top_k: Option<u32>,
    pub top_p: Option<f32>,
    pub num_predict: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaGenerateResponse {
    pub response: String,
    #[allow(dead_code)]
    pub done: Option<bool>,
    #[allow(dead_code)]
    pub total_duration: Option<u64>,
    pub prompt_eval_count: Option<u32>,
    pub prompt_eval_duration: Option<u64>,
    pub eval_count: Option<u32>,
    pub eval_duration: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaEmbedRequest {
    pub model: String,
    pub input: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaEmbedResponse {
    #[serde(default)]
    pub embeddings: Vec<Vec<f32>>,
}

pub(crate) fn ns_to_tps(tokens: f32, duration_ns: u64) -> Option<f32> {
    if duration_ns == 0 || tokens <= 0.0 {
        return None;
    }
    let seconds = duration_ns as f32 / 1_000_000_000.0;
    if seconds <= 0.0 {
        None
    } else {
        Some(tokens / seconds)
    }
}
