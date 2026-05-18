//! OpenAI-compatible HTTP request / response types.

use serde::{Deserialize, Serialize};

// ── Chat Completions ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatCompletionMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Optional tool definitions (ignored if the provider doesn't support them).
    #[serde(default)]
    pub tools: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    pub usage: ChatCompletionUsage,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatCompletionMessage,
    pub finish_reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// ── Streaming (SSE) ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatCompletionChunkChoice>,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionChunkChoice {
    pub index: u32,
    pub delta: ChatCompletionDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

// ── Models ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub object: &'static str,
    pub data: Vec<ModelObject>,
}

#[derive(Debug, Serialize)]
pub struct ModelObject {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: String,
}
