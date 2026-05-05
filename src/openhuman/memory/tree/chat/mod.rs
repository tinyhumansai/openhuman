//! Memory-tree chat backend abstraction.
//!
//! The memory_tree's two LLM consumers (the entity extractor and the
//! summariser) both want a small, structured "give me JSON for this prompt"
//! call. Historically each built its own `reqwest::Client` and talked to a
//! local Ollama daemon directly. This module replaces that with a single
//! [`ChatProvider`] trait so the same call site can be served by either:
//!
//! - **Cloud** — `providers::router` against the OpenHuman backend with
//!   the `summarization-v1` model. No local daemon required. Default for new
//!   installs.
//! - **Local** — the legacy Ollama-direct path. Opt-in via
//!   `memory_tree.llm_backend = "local"` in config or
//!   `OPENHUMAN_MEMORY_TREE_LLM_BACKEND=local`.
//!
//! ## Why a memory-tree-local trait
//!
//! The existing top-level [`crate::openhuman::providers::Provider`] trait
//! is rich (streaming, native tool calling, vision, …) and depends on the
//! agent's full conversation surface. The extractor and summariser only
//! need:
//!
//! 1. Send a (system, user) prompt pair.
//! 2. Get a JSON-shaped string back.
//!
//! Defining [`ChatProvider`] here keeps the memory_tree decoupled from
//! the agent's prompt/tool-calling stack, makes the extractor/summariser
//! trivial to mock in unit tests, and lets us route either the cloud or
//! the local backend through the same trait object.
//!
//! ## Soft-fallback contract
//!
//! Implementations of `chat_for_json` MUST NOT return `Err` for transient
//! upstream issues. Both memory_tree consumers fall back to a deterministic
//! no-op when the LLM is unavailable; bubbling the error up would abort
//! ingest cascades. Real bugs (e.g. malformed config) are still acceptable
//! `Err` cases — they should be rare and surfaced loudly.
//!
//! See [`local::OllamaChatProvider`] and [`cloud::CloudChatProvider`] for
//! the two production implementations.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::config::{Config, LlmBackend, DEFAULT_CLOUD_LLM_MODEL};

pub mod cloud;
pub mod local;

/// One pair of prompt messages handed to the chat backend.
///
/// Keeps the surface deliberately tiny — the memory_tree's two consumers
/// both build a system prompt + a single user message. Multi-turn,
/// streaming, and tool calling are out of scope.
#[derive(Debug, Clone)]
pub struct ChatPrompt {
    /// System prompt anchoring the model's role and expected output schema.
    pub system: String,
    /// User prompt carrying the dynamic input (the chunk text, the inputs
    /// to summarise, etc.).
    pub user: String,
    /// Sampling temperature. Both consumers use 0.0 today (max determinism).
    pub temperature: f64,
    /// Diagnostic tag included in tracing logs so seal-time and admit-time
    /// calls are easy to disambiguate. Stable, lowercase, no PII.
    pub kind: &'static str,
}

/// Pluggable chat surface used by the memory_tree's extractor + summariser.
///
/// Returns the model's raw output as a string. Callers parse it themselves
/// (typically as JSON conforming to a schema embedded in the system prompt)
/// because the parsing logic is consumer-specific.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Stable, grep-friendly name for logs. e.g. `"cloud:summarization-v1"`.
    fn name(&self) -> &str;

    /// Run one chat completion and return the assistant's content.
    ///
    /// Implementations should log entry / exit at debug level under the
    /// `[memory_tree::chat]` prefix.
    async fn chat_for_json(&self, prompt: &ChatPrompt) -> Result<String>;
}

/// Build the [`ChatProvider`] dictated by `config.memory_tree.llm_backend`.
///
/// - `Cloud` (default): wires [`cloud::CloudChatProvider`] against the
///   OpenHuman backend with `cloud_llm_model` (defaulting to
///   `summarization-v1`).
/// - `Local`: wires [`local::OllamaChatProvider`] against the legacy
///   `llm_extractor_endpoint` / `llm_extractor_model` config — the same
///   knobs that drove the Ollama-direct path before this refactor.
///
/// `consumer` is one of `"extract"` / `"summarise"` and selects the local
/// endpoint+model pair (extract uses `llm_extractor_*`, summarise uses
/// `llm_summariser_*`). For cloud both consumers share the same model.
pub fn build_chat_provider(
    config: &Config,
    consumer: ChatConsumer,
) -> Result<Arc<dyn ChatProvider>> {
    match config.memory_tree.llm_backend {
        LlmBackend::Cloud => {
            let model = config
                .memory_tree
                .cloud_llm_model
                .clone()
                .unwrap_or_else(|| DEFAULT_CLOUD_LLM_MODEL.to_string());
            // The `auth-profiles.json` lives next to `config.toml`, so the
            // openhuman_dir is the parent of config_path. Without this the
            // inner OpenHumanBackendProvider falls back to `~/.openhuman`
            // and fails with "No backend session" on any workspace not
            // located at the home default — the bug observed when running
            // with `OPENHUMAN_WORKSPACE` pointed elsewhere.
            let openhuman_dir = config.config_path.parent().map(std::path::PathBuf::from);
            log::debug!(
                "[memory_tree::chat] building Cloud provider consumer={} model={} \
                 openhuman_dir={:?}",
                consumer.as_str(),
                model,
                openhuman_dir
            );
            Ok(Arc::new(cloud::CloudChatProvider::new(
                config.api_url.clone(),
                model,
                openhuman_dir,
                config.secrets.encrypt,
            )))
        }
        LlmBackend::Local => {
            let (endpoint, model, timeout_ms) = match consumer {
                ChatConsumer::Extract => (
                    config.memory_tree.llm_extractor_endpoint.clone(),
                    config.memory_tree.llm_extractor_model.clone(),
                    config
                        .memory_tree
                        .llm_extractor_timeout_ms
                        .unwrap_or(15_000),
                ),
                ChatConsumer::Summarise => (
                    config.memory_tree.llm_summariser_endpoint.clone(),
                    config.memory_tree.llm_summariser_model.clone(),
                    config
                        .memory_tree
                        .llm_summariser_timeout_ms
                        .unwrap_or(120_000),
                ),
            };
            log::debug!(
                "[memory_tree::chat] building Local (Ollama) provider consumer={} \
                 endpoint_set={} model_set={} timeout_ms={}",
                consumer.as_str(),
                endpoint.is_some(),
                model.is_some(),
                timeout_ms
            );
            Ok(Arc::new(local::OllamaChatProvider::new(
                endpoint,
                model,
                std::time::Duration::from_millis(timeout_ms),
            )?))
        }
    }
}

/// Which memory-tree consumer is requesting a chat provider. Determines
/// which `llm_*_endpoint` / `llm_*_model` config fields are read in the
/// `Local` branch. Both consumers share the same cloud model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatConsumer {
    /// `LlmEntityExtractor` (per-chunk NER + importance rating).
    Extract,
    /// `LlmSummariser` (bucket-seal summary of N children).
    Summarise,
}

impl ChatConsumer {
    /// Stable wire string used in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extract => "extract",
            Self::Summarise => "summarise",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory chat provider for unit tests. Returns a canned response
    /// regardless of the prompt and counts invocations so tests can assert
    /// they were exercised.
    pub struct StaticChatProvider {
        pub response: String,
        pub calls: std::sync::atomic::AtomicUsize,
    }

    impl StaticChatProvider {
        pub fn new(response: impl Into<String>) -> Self {
            Self {
                response: response.into(),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ChatProvider for StaticChatProvider {
        fn name(&self) -> &str {
            "test:static"
        }
        async fn chat_for_json(&self, _prompt: &ChatPrompt) -> Result<String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    #[test]
    fn build_provider_returns_cloud_when_default() {
        let cfg = Config::default();
        // Default is LlmBackend::Cloud — provider construction must succeed
        // without a configured local Ollama endpoint.
        let provider = build_chat_provider(&cfg, ChatConsumer::Extract).unwrap();
        assert!(provider.name().contains("cloud"));
    }

    #[test]
    fn build_provider_returns_local_when_configured() {
        let mut cfg = Config::default();
        cfg.memory_tree.llm_backend = LlmBackend::Local;
        cfg.memory_tree.llm_extractor_endpoint = Some("http://localhost:11434".into());
        cfg.memory_tree.llm_extractor_model = Some("qwen2.5:0.5b".into());
        let provider = build_chat_provider(&cfg, ChatConsumer::Extract).unwrap();
        assert!(provider.name().contains("ollama") || provider.name().contains("local"));
    }

    #[test]
    fn chat_consumer_str_round_trip() {
        assert_eq!(ChatConsumer::Extract.as_str(), "extract");
        assert_eq!(ChatConsumer::Summarise.as_str(), "summarise");
    }

    #[tokio::test]
    async fn static_chat_provider_returns_response_and_counts() {
        let p = StaticChatProvider::new("hello");
        let prompt = ChatPrompt {
            system: "sys".into(),
            user: "u".into(),
            temperature: 0.0,
            kind: "test",
        };
        assert_eq!(p.chat_for_json(&prompt).await.unwrap(), "hello");
        assert_eq!(p.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
