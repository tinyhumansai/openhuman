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
    #[serde(default)]
    pub digest: Option<String>,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct OllamaPullProgress {
    layers: std::collections::BTreeMap<String, OllamaPullLayerProgress>,
    fallback_total: Option<u64>,
    fallback_completed: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct OllamaPullLayerProgress {
    total: Option<u64>,
    completed: u64,
}

impl OllamaPullProgress {
    pub(crate) fn observe(&mut self, event: &OllamaPullEvent) {
        if let Some(digest) = event
            .digest
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            let layer = self.layers.entry(digest.clone()).or_default();
            if let Some(total) = event.total {
                layer.total = Some(layer.total.unwrap_or(0).max(total));
                layer.completed = layer.completed.min(layer.total.unwrap_or(total));
            }
            if let Some(completed) = event.completed {
                let capped = layer
                    .total
                    .map(|total| completed.min(total))
                    .unwrap_or(completed);
                layer.completed = layer.completed.max(capped);
            }
            return;
        }

        if let Some(total) = event.total {
            self.fallback_total = Some(self.fallback_total.unwrap_or(0).max(total));
            self.fallback_completed = self
                .fallback_completed
                .min(self.fallback_total.unwrap_or(total));
        }
        if let Some(completed) = event.completed {
            let capped = self
                .fallback_total
                .map(|total| completed.min(total))
                .unwrap_or(completed);
            self.fallback_completed = self.fallback_completed.max(capped);
        }
    }

    pub(crate) fn aggregate_downloaded(&self) -> u64 {
        if !self.layers.is_empty() {
            return self.layers.values().map(|layer| layer.completed).sum();
        }
        self.fallback_completed
    }

    pub(crate) fn aggregate_total(&self) -> Option<u64> {
        if !self.layers.is_empty() {
            let mut total = 0_u64;
            let mut has_any = false;
            for layer in self.layers.values() {
                if let Some(layer_total) = layer.total {
                    total = total.saturating_add(layer_total);
                    has_any = true;
                }
            }
            return has_any.then_some(total);
        }
        self.fallback_total
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaTagsResponse {
    #[serde(default)]
    pub models: Vec<OllamaModelTag>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct OllamaModelTag {
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub modified_at: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct OllamaChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaGenerateOptions>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaChatResponse {
    pub message: OllamaChatMessage,
    #[allow(dead_code)]
    pub done: Option<bool>,
    pub prompt_eval_count: Option<u32>,
    pub prompt_eval_duration: Option<u64>,
    pub eval_count: Option<u32>,
    pub eval_duration: Option<u64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_progress_aggregates_layered_download_events() {
        let mut progress = OllamaPullProgress::default();

        progress.observe(&OllamaPullEvent {
            status: Some("pulling".to_string()),
            digest: Some("sha256:layer-a".to_string()),
            total: Some(100),
            completed: Some(20),
            error: None,
        });
        progress.observe(&OllamaPullEvent {
            status: Some("pulling".to_string()),
            digest: Some("sha256:layer-b".to_string()),
            total: Some(200),
            completed: Some(50),
            error: None,
        });
        progress.observe(&OllamaPullEvent {
            status: Some("pulling".to_string()),
            digest: Some("sha256:layer-a".to_string()),
            total: Some(100),
            completed: Some(100),
            error: None,
        });

        assert_eq!(progress.aggregate_downloaded(), 150);
        assert_eq!(progress.aggregate_total(), Some(300));
    }

    #[test]
    fn pull_progress_falls_back_when_digest_is_missing() {
        let mut progress = OllamaPullProgress::default();

        progress.observe(&OllamaPullEvent {
            status: Some("pulling manifest".to_string()),
            digest: None,
            total: Some(120),
            completed: Some(30),
            error: None,
        });
        progress.observe(&OllamaPullEvent {
            status: Some("pulling manifest".to_string()),
            digest: None,
            total: Some(120),
            completed: Some(80),
            error: None,
        });

        assert_eq!(progress.aggregate_downloaded(), 80);
        assert_eq!(progress.aggregate_total(), Some(120));
    }
}
