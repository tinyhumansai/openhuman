//! Ollama HTTP JSON types and small helpers (private to this crate).

use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

/// Hosts that serve the public Ollama website / model registry — never a local
/// or self-hosted Ollama API server. Pointing the base URL at one makes every
/// `/api/tags` poll hit ollama.com (HTTP 429 from its rate limiter), floods
/// Sentry with `list_models: non-success response`, and sends pointless traffic
/// to Ollama Inc. (TAURI-RUST-A3T).
///
/// Matched **exactly** (no suffix match) so a user's own self-hosted server at
/// e.g. `ollama.mycompany.com` is unaffected.
fn is_ollama_registry_host(host: &str) -> bool {
    matches!(
        host.trim()
            .trim_end_matches('.')
            .to_ascii_lowercase()
            .as_str(),
        "ollama.com" | "www.ollama.com" | "ollama.ai" | "www.ollama.ai" | "registry.ollama.ai"
    )
}

/// Returns `url` unless its host is a public Ollama registry host
/// ([`is_ollama_registry_host`]), in which case it logs a warning and falls back
/// to [`DEFAULT_OLLAMA_BASE_URL`]. This heals stale config/env values that
/// already target ollama.com (set before validation rejected it), so the
/// diagnostics poller stops hitting the website on the next launch.
fn reject_registry_host_or_default(url: String) -> String {
    let is_registry = reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(is_ollama_registry_host))
        .unwrap_or(false);
    if is_registry {
        log::warn!(
            "[local_ai] ignoring Ollama base URL {} (public website/registry, not a local server) — falling back to {DEFAULT_OLLAMA_BASE_URL}",
            redact_ollama_base_url(&url)
        );
        return DEFAULT_OLLAMA_BASE_URL.to_string();
    }
    url
}

/// Rewrite unspecified bind addresses (`0.0.0.0`, `[::]`) to their loopback
/// equivalents (`127.0.0.1`, `[::1]`).  Ollama's default `OLLAMA_HOST` is
/// `0.0.0.0:11434` — a valid *server-side* bind address but an invalid
/// *client-side* connect target on Windows (and misleading on other OSes).
fn normalize_unspecified_host(url: &str) -> String {
    if let Ok(parsed) = reqwest::Url::parse(url) {
        let replacement = match parsed.host() {
            Some(url::Host::Ipv4(addr)) if addr.is_unspecified() => Some("localhost"),
            Some(url::Host::Ipv6(addr)) if addr.is_unspecified() => Some("[::1]"),
            _ => None,
        };
        if let Some(new_host) = replacement {
            let scheme = parsed.scheme();
            let port_suffix = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
            let path = parsed.path().trim_end_matches('/');
            let result = format!("{scheme}://{new_host}{port_suffix}{path}");
            let result = result.trim_end_matches('/').to_string();
            log::debug!(
                "[local_ai] normalize_unspecified_host: rewrote {} -> {}",
                redact_ollama_base_url(url),
                redact_ollama_base_url(&result)
            );
            return result;
        }
    }
    url.to_string()
}

/// Returns the effective Ollama base URL.
///
/// Priority (highest to lowest):
/// 1. `OPENHUMAN_OLLAMA_BASE_URL` — app-specific override, used in tests.
/// 2. `OLLAMA_HOST` — Ollama's own env var; normalized to a full URL by
///    prepending `http://` when no scheme is present.
/// 3. [`DEFAULT_OLLAMA_BASE_URL`] — `http://localhost:11434`.
///
/// Unspecified bind addresses (`0.0.0.0`, `[::]`) are rewritten to their
/// loopback equivalents so the URL is valid as a client connect target.
pub(crate) fn ollama_base_url() -> String {
    if let Ok(url) = std::env::var("OPENHUMAN_OLLAMA_BASE_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return reject_registry_host_or_default(normalize_unspecified_host(
                trimmed.trim_end_matches('/'),
            ));
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
            return reject_registry_host_or_default(normalize_unspecified_host(&url));
        }
    }

    DEFAULT_OLLAMA_BASE_URL.to_string()
}

/// Returns the effective Ollama base URL, with `config.local_ai.base_url`
/// taking highest priority over env vars.
///
/// Priority (highest to lowest):
/// 1. `config.local_ai.base_url` if `Some` and non-empty (after trim)
/// 2. `OPENHUMAN_OLLAMA_BASE_URL` env var
/// 3. `OLLAMA_HOST` env var
/// 4. [`DEFAULT_OLLAMA_BASE_URL`]
///
/// Unspecified bind addresses (`0.0.0.0`, `[::]`) are rewritten to their
/// loopback equivalents so the URL is valid as a client connect target.
pub(crate) fn ollama_base_url_from_config(config: &crate::openhuman::config::Config) -> String {
    if let Some(ref url) = config.local_ai.base_url {
        let trimmed = url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            let normalized = reject_registry_host_or_default(normalize_unspecified_host(trimmed));
            log::debug!(
                "[local_ai] ollama_base_url_from_config: using config base_url -> {}",
                redact_ollama_base_url(&normalized)
            );
            return normalized;
        }
    }
    let resolved = ollama_base_url();
    log::debug!(
        "[local_ai] ollama_base_url_from_config: config base_url absent, resolved -> {}",
        redact_ollama_base_url(&resolved)
    );
    resolved
}

/// Validate and normalize a user-supplied Ollama URL.
///
/// - Trims whitespace and strips trailing slashes.
/// - Must have an `http://` or `https://` scheme.
/// - Must have a non-empty host.
/// - Rejects URLs with credentials (`user:pass@`).
/// - Rejects query strings and fragments.
/// - Strips any path component beyond root, normalizing to `scheme://host:port`.
///
/// Returns the normalized URL on success or an error message on failure.
pub(crate) fn validate_ollama_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("URL must not be empty".to_string());
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| format!("Invalid URL: {e}"))?;

    if parsed.host_str().map(|h| h.is_empty()).unwrap_or(true) {
        return Err("URL must have a non-empty host".to_string());
    }

    if parsed
        .host_str()
        .map(is_ollama_registry_host)
        .unwrap_or(false)
    {
        return Err(
            "ollama.com is the Ollama website, not a local server. Enter your local Ollama \
             address (e.g. http://localhost:11434) or your own server's URL."
                .to_string(),
        );
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL must not contain credentials (user:pass@host)".to_string());
    }

    if parsed.query().is_some() {
        return Err("URL must not contain a query string".to_string());
    }
    if parsed.fragment().is_some() {
        return Err("URL must not contain a fragment".to_string());
    }

    // Normalize to scheme://host[:port] — strip any path component.
    // Use the Host enum so IPv6 addresses are always re-bracketed correctly,
    // regardless of whether host_str() includes brackets in a given url-crate version.
    let host_formatted = match parsed.host() {
        Some(url::Host::Ipv4(addr)) if addr.is_unspecified() => "localhost".to_string(),
        Some(url::Host::Ipv6(addr)) if addr.is_unspecified() => "[::1]".to_string(),
        Some(url::Host::Ipv6(addr)) => format!("[{addr}]"),
        Some(h) => h.to_string(),
        None => String::new(),
    };
    let mut normalized = format!("{}://{}", parsed.scheme(), host_formatted);
    if let Some(port) = parsed.port() {
        normalized.push(':');
        normalized.push_str(&port.to_string());
    }

    log::debug!("[local_ai] validate_ollama_url: raw={trimmed:?} -> normalized={normalized:?}");
    Ok(normalized)
}

/// Strips userinfo, query, and fragment from `raw` so logs and error messages
/// don't leak `user:pass@host`-style credentials embedded in the endpoint.
pub(crate) fn redact_ollama_base_url(raw: &str) -> String {
    reqwest::Url::parse(raw)
        .map(|mut url| {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        })
        .unwrap_or_else(|_| "<invalid-endpoint>".to_string())
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

/// Resolved per-model signals from one Ollama `POST /api/show` round-trip.
///
/// Both fields are `None` when `/api/show` failed or omitted the data:
/// `context_length` → an `Unknown` memory-layer eligibility verdict;
/// `chat_capable` → "keep visible" in the chat picker (fail-open). See
/// [`OllamaShowResponse::chat_capability`] and Sentry TAURI-RUST-4P6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OllamaModelShow {
    pub context_length: Option<u64>,
    pub chat_capable: Option<bool>,
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
    /// Capability tags Ollama advertises for the model (e.g. `"completion"`,
    /// `"tools"`, `"vision"`, `"embedding"`, `"insert"`). Consumed by
    /// [`OllamaShowResponse::chat_capability`] to keep embedding-only models
    /// out of the chat-model picker (Sentry TAURI-RUST-4P6).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl OllamaShowResponse {
    /// Native context window (tokens) advertised by the model's GGUF
    /// metadata, or `None` when the server did not report it.
    pub(crate) fn context_length(&self) -> Option<u64> {
        context_length_from_model_info(&self.model_info)
    }

    /// Whether this model can serve chat/completions, from its `capabilities`.
    pub(crate) fn chat_capability(&self) -> Option<bool> {
        ollama_chat_capability(&self.capabilities)
    }
}

/// Classify whether an Ollama model can serve chat/completions from its
/// `/api/show` `capabilities` list.
///
/// Ollama tags text-generation models with `"completion"` (and newer builds
/// also `"chat"`); embedding models are tagged `"embedding"` only. We only
/// declare a model **not** chat-capable when we are confident it is
/// embedding-only — capabilities is non-empty, carries an embedding marker,
/// and carries no completion/chat marker. Anything ambiguous returns `None`
/// (unknown):
///   * empty / absent capabilities (older Ollama, or an `/api/show` miss);
///   * a tag set we don't recognise (e.g. `["insert"]` only).
/// Callers treat `None` as "keep visible" — fail-open, never hide a model
/// that might be usable for chat. Mirrors the non-rejecting `Unknown` arm of
/// [`super::model_requirements::ContextEligibility`]. See Sentry TAURI-RUST-4P6.
pub(crate) fn ollama_chat_capability(capabilities: &[String]) -> Option<bool> {
    if capabilities.is_empty() {
        return None;
    }
    let has = |needle: &str| {
        capabilities
            .iter()
            .any(|c| c.trim().eq_ignore_ascii_case(needle))
    };
    if has("completion") || has("chat") {
        Some(true)
    } else if has("embedding") || has("embed") {
        Some(false)
    } else {
        None
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
    fn chat_capability_classifies_embedding_completion_and_unknown() {
        let owned = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // Embedding-only model (bge-m3) → not chat-capable. TAURI-RUST-4P6.
        assert_eq!(ollama_chat_capability(&owned(&["embedding"])), Some(false));
        // Chat/completion models → chat-capable.
        assert_eq!(ollama_chat_capability(&owned(&["completion"])), Some(true));
        assert_eq!(
            ollama_chat_capability(&owned(&["completion", "tools", "vision"])),
            Some(true)
        );
        assert_eq!(ollama_chat_capability(&owned(&["chat"])), Some(true));
        // A model exposing BOTH stays chat-capable (completion wins).
        assert_eq!(
            ollama_chat_capability(&owned(&["embedding", "completion"])),
            Some(true)
        );
        // Unknown / fail-open: empty, or a tag set we don't recognise → None
        // (caller keeps the model visible).
        assert_eq!(ollama_chat_capability(&[]), None);
        assert_eq!(ollama_chat_capability(&owned(&["insert"])), None);
        // Case / whitespace tolerant.
        assert_eq!(
            ollama_chat_capability(&owned(&[" Embedding "])),
            Some(false)
        );
        assert_eq!(ollama_chat_capability(&owned(&["COMPLETION"])), Some(true));
    }

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

    // ── ollama_base_url_from_config ───────────────────────────────────

    fn make_config_with_base_url(url: Option<&str>) -> crate::openhuman::config::Config {
        let mut config = crate::openhuman::config::Config::default();
        config.local_ai.base_url = url.map(|s| s.to_string());
        config
    }

    #[test]
    fn ollama_base_url_from_config_takes_priority_over_env() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::set("http://127.0.0.1:55555");
        let config = make_config_with_base_url(Some("http://192.168.1.5:11434"));
        assert_eq!(
            ollama_base_url_from_config(&config),
            "http://192.168.1.5:11434"
        );
    }

    #[test]
    fn ollama_base_url_from_config_falls_back_when_none() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::set("http://127.0.0.1:55555");
        let config = make_config_with_base_url(None);
        assert_eq!(
            ollama_base_url_from_config(&config),
            "http://127.0.0.1:55555"
        );
    }

    // ── normalize_unspecified_host ──────────────────────────────────────

    #[test]
    fn normalize_rewrites_ipv4_unspecified() {
        assert_eq!(
            normalize_unspecified_host("http://0.0.0.0:11434"),
            "http://localhost:11434"
        );
    }

    #[test]
    fn normalize_rewrites_ipv6_unspecified() {
        assert_eq!(
            normalize_unspecified_host("http://[::]:11434"),
            "http://[::1]:11434"
        );
    }

    #[test]
    fn normalize_preserves_loopback() {
        assert_eq!(
            normalize_unspecified_host("http://127.0.0.1:11434"),
            "http://127.0.0.1:11434"
        );
        assert_eq!(
            normalize_unspecified_host("http://[::1]:11434"),
            "http://[::1]:11434"
        );
    }

    #[test]
    fn normalize_preserves_named_host() {
        assert_eq!(
            normalize_unspecified_host("http://localhost:11434"),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_unspecified_host("http://my-ollama.lan:11434"),
            "http://my-ollama.lan:11434"
        );
    }

    #[test]
    fn normalize_preserves_private_ip() {
        assert_eq!(
            normalize_unspecified_host("http://192.168.1.5:11434"),
            "http://192.168.1.5:11434"
        );
    }

    #[test]
    fn normalize_handles_invalid_url() {
        assert_eq!(normalize_unspecified_host("not a url"), "not a url");
    }

    // ── ollama_base_url: 0.0.0.0 normalization ─────────────────────────

    #[test]
    fn ollama_base_url_normalizes_unspecified_in_env_override() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::set("http://0.0.0.0:11434");
        assert_eq!(ollama_base_url(), "http://localhost:11434");
    }

    #[test]
    fn ollama_base_url_normalizes_unspecified_in_ollama_host() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "0.0.0.0:11434");
        assert_eq!(ollama_base_url(), "http://localhost:11434");
    }

    #[test]
    fn ollama_base_url_normalizes_ipv6_unspecified_in_ollama_host() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "http://[::]:11434");
        assert_eq!(ollama_base_url(), "http://[::1]:11434");
    }

    // ── ollama_base_url_from_config: 0.0.0.0 normalization ──────────────

    #[test]
    fn ollama_base_url_from_config_normalizes_unspecified() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::clear();
        let config = make_config_with_base_url(Some("http://0.0.0.0:11434"));
        assert_eq!(
            ollama_base_url_from_config(&config),
            "http://localhost:11434"
        );
    }

    #[test]
    fn ollama_base_url_from_config_normalizes_ipv6_unspecified() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::clear();
        let config = make_config_with_base_url(Some("http://[::]:11434"));
        assert_eq!(ollama_base_url_from_config(&config), "http://[::1]:11434");
    }

    // ── validate_ollama_url ───────────────────────────────────────────

    #[test]
    fn validate_ollama_url_accepts_http() {
        assert_eq!(
            validate_ollama_url("http://localhost:11434"),
            Ok("http://localhost:11434".to_string())
        );
    }

    #[test]
    fn validate_ollama_url_accepts_https() {
        assert_eq!(
            validate_ollama_url("https://remote-ollama.example.com:11434"),
            Ok("https://remote-ollama.example.com:11434".to_string())
        );
    }

    #[test]
    fn validate_ollama_url_rejects_no_scheme() {
        assert!(validate_ollama_url("localhost:11434").is_err());
        assert!(validate_ollama_url("ftp://localhost:11434").is_err());
    }

    #[test]
    fn validate_ollama_url_rejects_credentials() {
        assert!(validate_ollama_url("http://user:pass@localhost:11434").is_err());
    }

    #[test]
    fn validate_ollama_url_strips_path_and_normalizes() {
        assert_eq!(
            validate_ollama_url("http://192.168.1.5:11434/api/tags"),
            Ok("http://192.168.1.5:11434".to_string())
        );
    }

    #[test]
    fn validate_ollama_url_rejects_empty() {
        assert!(validate_ollama_url("").is_err());
        assert!(validate_ollama_url("   ").is_err());
    }

    #[test]
    fn validate_ollama_url_handles_ipv6() {
        assert_eq!(
            validate_ollama_url("http://[::1]:11434"),
            Ok("http://[::1]:11434".to_string())
        );
    }

    #[test]
    fn validate_ollama_url_rewrites_ipv4_unspecified_to_localhost() {
        assert_eq!(
            validate_ollama_url("http://0.0.0.0:11434"),
            Ok("http://localhost:11434".to_string())
        );
    }

    #[test]
    fn validate_ollama_url_rewrites_ipv6_unspecified_to_loopback() {
        assert_eq!(
            validate_ollama_url("http://[::]:11434"),
            Ok("http://[::1]:11434".to_string())
        );
    }

    // ── public Ollama registry-host rejection (TAURI-RUST-A3T) ─────────

    #[test]
    fn is_ollama_registry_host_matches_public_hosts_case_insensitively() {
        for host in [
            "ollama.com",
            "OLLAMA.COM",
            "www.ollama.com",
            "ollama.ai",
            "www.ollama.ai",
            "registry.ollama.ai",
            "ollama.com.", // trailing FQDN dot
        ] {
            assert!(is_ollama_registry_host(host), "expected reject: {host}");
        }
    }

    #[test]
    fn is_ollama_registry_host_allows_self_hosted_and_loopback() {
        // No suffix match: a user's own subdomain must NOT be blocked.
        for host in [
            "localhost",
            "127.0.0.1",
            "ollama.mycompany.com",
            "my-ollama.lan",
            "remote-ollama.example.com",
            "notollama.com",
        ] {
            assert!(!is_ollama_registry_host(host), "expected allow: {host}");
        }
    }

    #[test]
    fn validate_ollama_url_rejects_public_registry_hosts() {
        // The bug: ollama.com is the website/registry, not a local server.
        // `/api/tags` against it returns HTTP 429 and floods Sentry.
        for url in [
            "https://ollama.com",
            "http://ollama.com",
            "https://ollama.com/",
            "https://www.ollama.com:443",
            "https://ollama.ai",
            "https://registry.ollama.ai",
        ] {
            let err = validate_ollama_url(url).unwrap_err();
            assert!(
                err.contains("ollama.com is the Ollama website"),
                "expected website rejection for {url}, got: {err}"
            );
        }
    }

    #[test]
    fn validate_ollama_url_still_accepts_self_hosted_subdomain() {
        assert_eq!(
            validate_ollama_url("https://ollama.mycompany.com:11434"),
            Ok("https://ollama.mycompany.com:11434".to_string())
        );
    }

    #[test]
    fn ollama_base_url_from_config_heals_registry_host_to_default() {
        // Existing users who saved ollama.com before validation rejected it
        // must stop hitting the website on the next launch.
        let _lock = test_lock();
        let _g = OllamaEnvGuard::clear();
        let config = make_config_with_base_url(Some("https://ollama.com"));
        assert_eq!(
            ollama_base_url_from_config(&config),
            DEFAULT_OLLAMA_BASE_URL
        );
    }

    #[test]
    fn ollama_base_url_heals_registry_host_from_env_override() {
        let _lock = test_lock();
        let _g = OllamaEnvGuard::set("https://ollama.com");
        assert_eq!(ollama_base_url(), DEFAULT_OLLAMA_BASE_URL);
    }

    #[test]
    fn ollama_base_url_heals_registry_host_from_ollama_host() {
        let _lock = test_lock();
        let _g1 = OllamaEnvGuard::clear();
        let _g2 = OllamaEnvGuard::set_var(OLLAMA_HOST_VAR, "ollama.com");
        assert_eq!(ollama_base_url(), DEFAULT_OLLAMA_BASE_URL);
    }

    // ── redact_ollama_base_url ────────────────────────────────────────

    #[test]
    fn redact_strips_userinfo_query_and_fragment() {
        assert_eq!(
            redact_ollama_base_url("http://user:pass@host:11434/api?token=abc#frag"),
            "http://host:11434/api"
        );
    }

    #[test]
    fn redact_keeps_plain_url() {
        assert_eq!(
            redact_ollama_base_url("http://127.0.0.1:11434/"),
            "http://127.0.0.1:11434/"
        );
    }

    #[test]
    fn redact_handles_invalid_url() {
        assert_eq!(redact_ollama_base_url("not a url"), "<invalid-endpoint>");
    }
}
