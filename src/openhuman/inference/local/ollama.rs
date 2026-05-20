//! Ollama HTTP JSON types and small helpers (private to this crate).

use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

/// Returns the effective Ollama base URL.
///
/// Priority (highest to lowest):
/// 1. `OPENHUMAN_OLLAMA_BASE_URL` — app-specific override, used in tests.
/// 2. `OLLAMA_HOST` — Ollama's own env var; normalized to a full URL by
///    prepending `http://` when no scheme is present.
/// 3. [`DEFAULT_OLLAMA_BASE_URL`] — `http://localhost:11434`.
pub(crate) fn ollama_base_url() -> String {
    if let Ok(url) = std::env::var("OPENHUMAN_OLLAMA_BASE_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return trimmed.trim_end_matches('/').to_string();
        }
    }

    if let Ok(host) = std::env::var("OLLAMA_HOST") {
        let trimmed = host.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            let url = if trimmed.contains("://") {
                trimmed.to_string()
            } else {
                format!("http://{trimmed}")
            };
            log::debug!("[local_ai] ollama_base_url: using OLLAMA_HOST -> {url}");
            return url;
        }
    }

    DEFAULT_OLLAMA_BASE_URL.to_string()
}

/// Back-compat constant kept at its original value for callers that
/// reference it directly. New callers should use [`ollama_base_url`].
pub(crate) const OLLAMA_BASE_URL: &str = DEFAULT_OLLAMA_BASE_URL;

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

/// Request body for Ollama `POST /api/show`.
#[derive(Debug, Serialize)]
pub(crate) struct OllamaShowRequest {
    /// Model tag (e.g. `bge-m3:latest`). `model` is the current Ollama
    /// field; older servers also accept the legacy `name`, which `model`
    /// is forward-compatible with.
    pub model: String,
}

/// Subset of Ollama `POST /api/show` we consume.
///
/// `model_info` is an open key/value map whose keys are architecture-scoped
/// (e.g. `llama.context_length`, `bert.embedding_length`). The prefix is
/// whatever `general.architecture` reports, so the context-length key is
/// resolved dynamically rather than hard-coded per model family.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct OllamaShowResponse {
    #[serde(default)]
    pub model_info: serde_json::Map<String, serde_json::Value>,
    // Present in the Ollama API response; retained to document the wire shape, not yet consumed.
    #[serde(default)]
    #[allow(dead_code)]
    pub capabilities: Vec<String>,
}

impl OllamaShowResponse {
    /// Native context window (tokens) advertised by the model's GGUF
    /// metadata, or `None` when the server did not report it.
    pub(crate) fn context_length(&self) -> Option<u64> {
        context_length_from_model_info(&self.model_info)
    }
}

/// Extract `<arch>.context_length` from an Ollama `model_info` map.
///
/// Resolution order:
/// 1. `general.architecture` → `{arch}.context_length` (documented shape).
/// 2. Fallback: the first key ending in `.context_length` — covers servers
///    that omit `general.architecture` or use an unexpected prefix.
pub(crate) fn context_length_from_model_info(
    info: &serde_json::Map<String, serde_json::Value>,
) -> Option<u64> {
    fn as_u64(v: &serde_json::Value) -> Option<u64> {
        v.as_u64()
            .or_else(|| v.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
            .or_else(|| v.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
    }
    if let Some(arch) = info.get("general.architecture").and_then(|v| v.as_str()) {
        if let Some(value) = info.get(&format!("{arch}.context_length")).and_then(as_u64) {
            return Some(value);
        }
    }
    info.iter()
        .filter(|(key, _)| key.ends_with(".context_length"))
        .filter_map(|(_, value)| as_u64(value))
        .max()
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

    // ── /api/show context-length extraction ──────────────────────────

    fn show_response(json: serde_json::Value) -> OllamaShowResponse {
        serde_json::from_value(json).expect("OllamaShowResponse")
    }

    #[test]
    fn context_length_uses_general_architecture_prefix() {
        let resp = show_response(serde_json::json!({
            "model_info": {
                "general.architecture": "bert",
                "bert.context_length": 8192,
                "bert.embedding_length": 1024
            }
        }));
        assert_eq!(resp.context_length(), Some(8192));
    }

    #[test]
    fn context_length_falls_back_when_architecture_missing() {
        let resp = show_response(serde_json::json!({
            "model_info": { "llama.context_length": 4096 }
        }));
        assert_eq!(resp.context_length(), Some(4096));
    }

    #[test]
    fn context_length_handles_float_and_string_encodings() {
        // Some servers serialize the metadata number as a float.
        let float = show_response(serde_json::json!({
            "model_info": { "general.architecture": "qwen2", "qwen2.context_length": 32768.0 }
        }));
        assert_eq!(float.context_length(), Some(32768));

        // Non-numeric / missing → None (caller treats as Unknown, not a hard fail).
        let missing = show_response(serde_json::json!({ "model_info": {} }));
        assert_eq!(missing.context_length(), None);
        let absent_field = show_response(serde_json::json!({}));
        assert_eq!(absent_field.context_length(), None);
    }

    #[test]
    fn context_length_prefers_architecture_key_over_unrelated_match() {
        let resp = show_response(serde_json::json!({
            "model_info": {
                "general.architecture": "llama",
                "llama.context_length": 8192,
                "clip.context_length": 77
            }
        }));
        assert_eq!(resp.context_length(), Some(8192));
    }

    #[test]
    fn context_length_fallback_returns_max_not_first() {
        // Without `general.architecture`, the fallback must pick the *largest*
        // `.context_length` value, not the first one encountered. Multimodal
        // models can carry a low secondary value (e.g. `clip.context_length:77`)
        // which, if chosen first, would incorrectly mark the model below minimum.
        let resp = show_response(serde_json::json!({
            "model_info": {
                "clip.context_length": 77,
                "llama.context_length": 32768
            }
        }));
        assert_eq!(resp.context_length(), Some(32768));
    }

    // ── ollama_base_url env-override behaviour ───────────────────────
    //
    // These tests mutate the process-global `OPENHUMAN_OLLAMA_BASE_URL`
    // variable, so they coordinate with the shared `LOCAL_AI_TEST_MUTEX`
    // used by `public_infer.rs` tests to prevent interleaved set/remove
    // calls from other tests in the same binary.

    const ENV_VAR: &str = "OPENHUMAN_OLLAMA_BASE_URL";
    const OLLAMA_HOST_VAR: &str = "OLLAMA_HOST";

    struct OllamaEnvGuard {
        var: &'static str,
        prior: Option<String>,
    }

    impl OllamaEnvGuard {
        fn clear() -> Self {
            let prior = std::env::var(ENV_VAR).ok();
            unsafe { std::env::remove_var(ENV_VAR) };
            Self {
                var: ENV_VAR,
                prior,
            }
        }

        fn set(value: &str) -> Self {
            let prior = std::env::var(ENV_VAR).ok();
            unsafe { std::env::set_var(ENV_VAR, value) };
            Self {
                var: ENV_VAR,
                prior,
            }
        }

        fn clear_var(var: &'static str) -> Self {
            let prior = std::env::var(var).ok();
            unsafe { std::env::remove_var(var) };
            Self { var, prior }
        }

        fn set_var(var: &'static str, value: &str) -> Self {
            let prior = std::env::var(var).ok();
            unsafe { std::env::set_var(var, value) };
            Self { var, prior }
        }
    }

    impl Drop for OllamaEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prior.take() {
                    Some(v) => std::env::set_var(self.var, v),
                    None => std::env::remove_var(self.var),
                }
            }
        }
    }

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::inference::inference_test_guard()
    }

    #[test]
    fn ollama_base_url_returns_default_when_env_unset() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::clear();
        assert_eq!(ollama_base_url(), DEFAULT_OLLAMA_BASE_URL);
    }

    #[test]
    fn ollama_base_url_returns_env_value_for_normal_url() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::set("http://127.0.0.1:55555");
        assert_eq!(ollama_base_url(), "http://127.0.0.1:55555");
    }

    #[test]
    fn ollama_base_url_trims_surrounding_whitespace() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::set("   http://127.0.0.1:55555   ");
        assert_eq!(ollama_base_url(), "http://127.0.0.1:55555");
    }

    #[test]
    fn ollama_base_url_strips_trailing_slashes() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::set("http://127.0.0.1:55555///");
        assert_eq!(ollama_base_url(), "http://127.0.0.1:55555");
    }

    #[test]
    fn ollama_base_url_falls_back_for_empty_or_whitespace_env() {
        let _lock = test_lock();
        {
            let _g = OllamaEnvGuard::set("");
            assert_eq!(ollama_base_url(), DEFAULT_OLLAMA_BASE_URL);
        }
        {
            let _g = OllamaEnvGuard::set("   ");
            assert_eq!(ollama_base_url(), DEFAULT_OLLAMA_BASE_URL);
        }
    }

    #[test]
    fn ollama_base_url_uses_ollama_host_when_openhuman_var_unset() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "192.168.1.5:11434");
        assert_eq!(ollama_base_url(), "http://192.168.1.5:11434");
    }

    #[test]
    fn ollama_base_url_prepends_http_for_host_without_scheme() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "myhost:11434");
        assert_eq!(ollama_base_url(), "http://myhost:11434");
    }

    #[test]
    fn ollama_base_url_preserves_existing_scheme_in_ollama_host() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "https://remote-ollama.example.com");
        assert_eq!(ollama_base_url(), "https://remote-ollama.example.com");
    }

    #[test]
    fn ollama_base_url_openhuman_var_takes_priority_over_ollama_host() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::set("http://127.0.0.1:55555");
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "192.168.1.5:11434");
        assert_eq!(ollama_base_url(), "http://127.0.0.1:55555");
    }

    #[test]
    fn ollama_base_url_ignores_empty_ollama_host() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "   ");
        assert_eq!(ollama_base_url(), DEFAULT_OLLAMA_BASE_URL);
    }

    #[test]
    fn ollama_base_url_strips_trailing_slash_from_ollama_host() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "myhost:11434/");
        assert_eq!(ollama_base_url(), "http://myhost:11434");
    }
}
