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
    create_chat_provider, provider_for_role, ChatMessage, ChatRequest, Provider, UsageInfo,
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

    /// Like [`chat_for_text`], but also surfaces the provider-reported
    /// [`UsageInfo`] (real token counts + `charged_amount_usd`) when the
    /// backing provider returns it.
    ///
    /// The default implementation simply runs `chat_for_text` and reports
    /// `None` usage, so implementors (e.g. test doubles, external impls)
    /// that don't thread usage keep compiling unchanged. The production
    /// `InferenceChatProvider` overrides this to route through the
    /// inference `Provider::chat` API, which already parses usage out of
    /// the backend response (see `compatible::extract_usage`).
    async fn chat_for_text_with_usage(
        &self,
        prompt: &ChatPrompt,
    ) -> Result<(String, Option<UsageInfo>)> {
        let text = self.chat_for_text(prompt).await?;
        Ok((text, None))
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
        let (text, _usage) = self.run_with_usage(prompt).await?;
        Ok(text)
    }

    /// Run the prompt through the inference `Provider::chat` API and return
    /// both the text and the provider-reported usage. Memory historically
    /// called `chat_with_history` (which returns only `String` and drops
    /// the parsed usage); routing through `chat` instead lets us thread the
    /// real token counts + `charged_amount_usd` into the sync audit log
    /// (issue #3110) without re-deriving them from `body.len() / 4`.
    async fn run_with_usage(&self, prompt: &ChatPrompt) -> Result<(String, Option<UsageInfo>)> {
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

        let request = ChatRequest {
            messages: &messages,
            tools: None,
            stream: None,
        };

        let response = self
            .inner
            .chat(request, &self.model, prompt.temperature)
            .await?;

        // Fail fast on a missing body rather than masking it as an empty
        // string: an empty summary would still be ingested (and, post-#3110,
        // counted against the run's real charge) as if it were valid output.
        // The caller's fallback path (`fallback_summary`) is the correct
        // recovery for a silent provider, and it only runs on `Err`.
        let Some(text) = response.text else {
            anyhow::bail!(
                "inference provider '{}' returned no text for {} summarise request",
                self.display,
                prompt.kind
            );
        };
        let usage = response.usage;

        log::debug!(
            "[memory::chat] provider={} kind={} response_chars={} usage_present={} input_tokens={} output_tokens={} charged_usd={}",
            self.display,
            prompt.kind,
            text.len(),
            usage.is_some(),
            usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
            usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
            usage.as_ref().map(|u| u.charged_amount_usd).unwrap_or(0.0),
        );

        Ok((text, usage))
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

    async fn chat_for_text_with_usage(
        &self,
        prompt: &ChatPrompt,
    ) -> Result<(String, Option<UsageInfo>)> {
        self.run_with_usage(prompt).await
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

    #[tokio::test]
    async fn chat_for_text_with_usage_default_impl_reports_no_usage() {
        // A provider that doesn't override `chat_for_text_with_usage`
        // (here the `chat_for_json`-only `StaticChatProvider`) must still
        // return its text, with `None` usage — so summarise() falls back
        // to the estimate rather than reporting a bogus zero charge.
        let p = StaticChatProvider::new("summary text");
        let prompt = ChatPrompt {
            system: "sys".into(),
            user: "u".into(),
            temperature: 0.0,
            kind: "test",
        };
        let (text, usage) = p.chat_for_text_with_usage(&prompt).await.unwrap();
        assert_eq!(text, "summary text");
        assert!(usage.is_none());
    }
}
