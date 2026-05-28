//! Memory LLM adapter backed by the unified inference provider stack.
//!
//! Memory callers still want a tiny prompt surface: one system message, one
//! user message, and a string response. This module keeps that narrow contract
//! for the rest of the memory layer, but routes every production call through
//! `openhuman::inference::provider` so memory uses the same workload routing as
//! the rest of the app.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::config::{Config, DEFAULT_CLOUD_LLM_MODEL};
use crate::openhuman::inference::provider::{
    create_chat_provider, provider_for_role, ChatMessage, Provider,
};

/// One pair of prompt messages handed to the memory LLM backend.
#[derive(Debug, Clone)]
pub struct ChatPrompt {
    pub system: String,
    pub user: String,
    pub temperature: f64,
    pub kind: &'static str,
}

/// Pluggable LLM surface used by the memory layer.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn chat_for_json(&self, prompt: &ChatPrompt) -> Result<String>;

    async fn chat_for_text(&self, prompt: &ChatPrompt) -> Result<String> {
        self.chat_for_json(prompt).await
    }
}

struct InferenceChatProvider {
    inner: Box<dyn Provider>,
    model: String,
    display: String,
}

impl InferenceChatProvider {
    fn new(inner: Box<dyn Provider>, model: String) -> Self {
        let display = format!("inference:{model}");
        Self {
            inner,
            model,
            display,
        }
    }

    async fn run(&self, prompt: &ChatPrompt) -> Result<String> {
        log::debug!(
            "[memory::chat] provider={} kind={} model={} sys_chars={} user_chars={}",
            self.display,
            prompt.kind,
            self.model,
            prompt.system.len(),
            prompt.user.len()
        );

        let messages = vec![
            ChatMessage::system(prompt.system.clone()),
            ChatMessage::user(prompt.user.clone()),
        ];

        let text = self
            .inner
            .chat_with_history(&messages, &self.model, prompt.temperature)
            .await?;

        log::debug!(
            "[memory::chat] provider={} kind={} response_chars={}",
            self.display,
            prompt.kind,
            text.len()
        );

        Ok(text)
    }
}

#[async_trait]
impl ChatProvider for InferenceChatProvider {
    fn name(&self) -> &str {
        &self.display
    }

    async fn chat_for_json(&self, prompt: &ChatPrompt) -> Result<String> {
        self.run(prompt).await
    }

    async fn chat_for_text(&self, prompt: &ChatPrompt) -> Result<String> {
        self.run(prompt).await
    }
}

fn routed_memory_config(config: &Config) -> Config {
    let mut routed = config.clone();
    if !config.workload_uses_local("memory") {
        routed.default_model = Some(
            config
                .memory_tree
                .cloud_llm_model
                .clone()
                .unwrap_or_else(|| DEFAULT_CLOUD_LLM_MODEL.to_string()),
        );
    }
    routed
}

#[cfg(test)]
fn test_override_runtime() -> Option<(Arc<dyn ChatProvider>, String)> {
    test_override::current().map(|provider| (provider, "test:override".to_string()))
}

#[cfg(not(test))]
fn test_override_runtime() -> Option<(Arc<dyn ChatProvider>, String)> {
    None
}

/// Build the memory LLM provider and return the resolved model id.
pub fn build_chat_runtime(config: &Config) -> Result<(Arc<dyn ChatProvider>, String)> {
    if let Some(runtime) = test_override_runtime() {
        return Ok(runtime);
    }

    let routed = routed_memory_config(config);
    let resolved_provider = provider_for_role("summarization", &routed);
    let (provider, model) = create_chat_provider("summarization", &routed)?;

    log::debug!(
        "[memory::chat] built provider route={} model={}",
        resolved_provider,
        model
    );

    Ok((
        Arc::new(InferenceChatProvider::new(provider, model.clone())),
        model,
    ))
}

/// Build the memory LLM provider dictated by the inference workload routing.
pub fn build_chat_provider(config: &Config) -> Result<Arc<dyn ChatProvider>> {
    Ok(build_chat_runtime(config)?.0)
}

#[cfg(test)]
pub struct StaticChatProvider {
    pub response: String,
    pub calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl StaticChatProvider {
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[cfg(test)]
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

#[cfg(test)]
pub mod test_override {
    use super::ChatProvider;
    use std::sync::Arc;

    tokio::task_local! {
        static OVERRIDE: Arc<dyn ChatProvider>;
    }

    pub fn current() -> Option<Arc<dyn ChatProvider>> {
        OVERRIDE.try_with(Arc::clone).ok()
    }

    pub async fn with_provider<F, T>(provider: Arc<dyn ChatProvider>, fut: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        OVERRIDE.scope(provider, fut).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_provider_returns_inference_wrapper_when_default() {
        let cfg = Config::default();
        let provider = build_chat_provider(&cfg).unwrap();
        assert!(provider.name().contains("inference:"));
    }

    #[test]
    fn build_chat_runtime_defaults_to_openhuman_resolved_model() {
        let cfg = Config::default();
        let (_provider, model) = build_chat_runtime(&cfg).unwrap();
        assert_eq!(model, DEFAULT_CLOUD_LLM_MODEL);
        // build_chat_runtime resolves the "summarization" workload role,
        // which routes to the dedicated DEFAULT_CLOUD_LLM_MODEL
        // (`summarization-v1`, PR #2690) rather than the generic
        // `reasoning-v1` fallback.
        assert_eq!(model, DEFAULT_CLOUD_LLM_MODEL);
    }

    #[test]
    fn build_chat_runtime_still_builds_when_cloud_memory_model_is_overridden() {
        let mut cfg = Config::default();
        cfg.memory_tree.cloud_llm_model = Some("custom-summary-model".into());
        let (_provider, model) = build_chat_runtime(&cfg).unwrap();
        // Setting memory_tree.cloud_llm_model overrides the cloud-memory
        // model path; the routing falls back to the platform default
        // (`reasoning-v1`) rather than the `summarization-v1` tier.
        assert_eq!(model, "reasoning-v1");
    }

    #[test]
    fn build_provider_returns_inference_wrapper_when_local_memory_is_configured() {
        let mut cfg = Config::default();
        cfg.memory_provider = Some("ollama:qwen2.5:0.5b".into());
        let provider = build_chat_provider(&cfg).unwrap();
        assert!(provider.name().contains("qwen2.5:0.5b"));
    }

    #[test]
    fn build_chat_runtime_preserves_local_memory_model() {
        let mut cfg = Config::default();
        cfg.memory_provider = Some("ollama:qwen2.5:0.5b".into());
        let (_provider, model) = build_chat_runtime(&cfg).unwrap();
        assert_eq!(model, "qwen2.5:0.5b");
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
