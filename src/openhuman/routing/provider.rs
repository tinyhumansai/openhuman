//! Policy-driven provider that routes requests between local and remote models.
//!
//! [`IntelligentRoutingProvider`] implements the [`Provider`] trait. On each call:
//!
//! 1. Classifies the `hint:*` model string → [`TaskCategory`].
//! 2. Checks local Ollama health (cached, non-blocking).
//! 3. Applies routing policy (task category + [`RoutingHints`]).
//! 4. Calls the chosen provider; captures latency and token usage.
//! 5. If local was chosen and:
//!    - call **fails** → fallback to remote (unless `privacy_required`).
//!    - call **succeeds but quality is low** → fallback to remote (same guard).
//! 6. Emits a [`RoutingRecord`] via structured tracing for every completed call.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::config::{MODEL_AGENTIC_V1, MODEL_CODING_V1, MODEL_REASONING_V1};
use crate::openhuman::providers::traits::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ProviderCapabilities, StreamChunk,
    StreamError, StreamOptions, StreamResult, ToolsPayload,
};
use crate::openhuman::tools::ToolSpec;

use super::health::LocalHealthChecker;
use super::policy::{self, RoutingHints, RoutingTarget, TaskCategory};
use super::quality;
use super::telemetry::{self, RoutingRecord};

fn stream_local_not_supported_error() -> StreamResult<StreamChunk> {
    Err(StreamError::Provider(
        "[routing] streaming selected local path, but local streaming is not implemented"
            .to_string(),
    ))
}

fn truncate_safe(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }

    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn should_fallback(
    result: &Result<ChatResponse>,
    privacy_required: bool,
    fallback: &Option<RoutingTarget>,
) -> bool {
    if privacy_required || fallback.is_none() {
        return false;
    }

    match result {
        Err(_) => true,
        Ok(resp) => quality::is_low_quality(resp.text.as_deref().unwrap_or("")),
    }
}

/// Provider that routes requests between a local Ollama instance and the remote
/// OpenHuman backend based on task complexity, local health, and routing hints.
pub struct IntelligentRoutingProvider {
    remote: Box<dyn Provider>,
    local: Box<dyn Provider>,
    local_model: String,
    /// Model string sent to remote on fallback (e.g. configured default model).
    remote_fallback_model: String,
    /// Mirrors `config.local_ai.enabled`.
    local_enabled: bool,
    health: Arc<LocalHealthChecker>,
    /// Global routing hints (privacy, latency, cost).
    hints: RoutingHints,
}

impl IntelligentRoutingProvider {
    fn resolve_streaming_target(&self, model: &str) -> (RoutingTarget, String) {
        let category = policy::classify(model);
        let remote_model = self.resolve_remote_model(model, category);
        let (primary, _fallback) = policy::decide(
            category,
            &self.local_model,
            &remote_model,
            self.local_enabled,
            &self.hints,
        );
        (primary, remote_model)
    }

    fn resolve_remote_model(&self, requested_model: &str, category: TaskCategory) -> String {
        if category != TaskCategory::Heavy {
            return self.remote_fallback_model.clone();
        }

        // Keep remote model naming aligned with backend modelRegistry.
        match requested_model.strip_prefix("hint:") {
            Some("reasoning") => MODEL_REASONING_V1.to_string(),
            Some("agentic") => MODEL_AGENTIC_V1.to_string(),
            Some("coding") => MODEL_CODING_V1.to_string(),
            _ => requested_model.to_string(),
        }
    }

    pub fn new(
        remote: Box<dyn Provider>,
        local: Box<dyn Provider>,
        local_model: String,
        remote_fallback_model: String,
        local_enabled: bool,
        health: Arc<LocalHealthChecker>,
    ) -> Self {
        Self::with_hints(
            remote,
            local,
            local_model,
            remote_fallback_model,
            local_enabled,
            health,
            RoutingHints::default(),
        )
    }

    /// Same as [`new`] but with caller-supplied routing hints.
    pub fn with_hints(
        remote: Box<dyn Provider>,
        local: Box<dyn Provider>,
        local_model: String,
        remote_fallback_model: String,
        local_enabled: bool,
        health: Arc<LocalHealthChecker>,
        hints: RoutingHints,
    ) -> Self {
        Self {
            remote,
            local,
            local_model,
            remote_fallback_model,
            local_enabled,
            health,
            hints,
        }
    }

    /// Resolve routing targets for the given model string.
    ///
    /// Returns `(primary, fallback, category, local_healthy)`.
    async fn resolve(
        &self,
        model: &str,
    ) -> (RoutingTarget, Option<RoutingTarget>, TaskCategory, bool) {
        let category = policy::classify(model);

        let local_healthy = if self.local_enabled {
            self.health.is_healthy().await
        } else {
            false
        };

        // Heavy hint models are normalized to backend-valid model IDs.
        // Lightweight/medium fallbacks use the configured default remote model.
        let remote_model = self.resolve_remote_model(model, category);

        let (primary, fallback) = policy::decide(
            category,
            &self.local_model,
            &remote_model,
            local_healthy,
            &self.hints,
        );

        (primary, fallback, category, local_healthy)
    }

    /// Attempt a local call; on error or low quality (and when fallback is
    /// available), transparently retry with remote.
    async fn try_local_with_fallback(
        &self,
        local_call: impl std::future::Future<Output = Result<String>>,
        fallback: &Option<RoutingTarget>,
        fallback_fn: impl std::future::Future<Output = Result<String>>,
        hint: &str,
        privacy_required: bool,
    ) -> (Result<String>, bool) {
        let result = local_call.await;

        match &result {
            Err(e) => {
                if !privacy_required {
                    if let Some(RoutingTarget::Remote { .. }) = fallback {
                        tracing::warn!(
                            hint,
                            error = ?e,
                            "[routing] local call failed, retrying with remote"
                        );
                        return (fallback_fn.await, true);
                    }
                }
                (result, false)
            }
            Ok(text) if !privacy_required && quality::is_low_quality(text) => {
                if let Some(RoutingTarget::Remote { .. }) = fallback {
                    tracing::warn!(
                        hint,
                        response_preview = truncate_safe(text, 80),
                        "[routing] local response low quality, retrying with remote"
                    );
                    return (fallback_fn.await, true);
                }
                (result, false)
            }
            _ => (result, false),
        }
    }

    async fn dispatch_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> Result<String> {
        let (primary, fallback, category, local_healthy) = self.resolve(model).await;
        let started = Instant::now();

        let (result, fallback_occurred) = match &primary {
            RoutingTarget::Local { model: m } => {
                tracing::debug!(model = m.as_str(), hint = model, "[routing] → local");
                let m = m.clone();
                let fb_model = fallback
                    .as_ref()
                    .and_then(|t| {
                        if let RoutingTarget::Remote { model } = t {
                            Some(model.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                self.try_local_with_fallback(
                    self.local
                        .chat_with_system(system_prompt, message, &m, temperature),
                    &fallback,
                    self.remote
                        .chat_with_system(system_prompt, message, &fb_model, temperature),
                    model,
                    self.hints.privacy_required,
                )
                .await
            }
            RoutingTarget::Remote { model: m } => {
                tracing::debug!(model = m.as_str(), hint = model, "[routing] → remote");
                (
                    self.remote
                        .chat_with_system(system_prompt, message, m, temperature)
                        .await,
                    false,
                )
            }
        };

        telemetry::emit(&RoutingRecord {
            model_hint: model.to_string(),
            task_category: category.as_str(),
            routed_to: if fallback_occurred {
                "remote"
            } else {
                primary.label()
            },
            resolved_model: if fallback_occurred {
                fallback
                    .as_ref()
                    .map(|t| t.model().to_string())
                    .unwrap_or_default()
            } else {
                primary.model().to_string()
            },
            local_healthy,
            fallback_to_remote: fallback_occurred,
            latency_ms: started.elapsed().as_millis() as u64,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        });

        result
    }

    async fn dispatch_chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> Result<ChatResponse> {
        let has_tools = request.tools.map_or(false, |t| !t.is_empty());
        let (primary, fallback, category, local_healthy) = self.resolve(model).await;
        let started = Instant::now();
        let mut fallback_occurred = false;

        // Tools require native tool calling — always force remote.
        let effective_primary = if has_tools && matches!(primary, RoutingTarget::Local { .. }) {
            tracing::debug!(hint = model, "[routing] tools present → remote");
            RoutingTarget::Remote {
                model: self.remote_fallback_model.clone(),
            }
        } else {
            primary.clone()
        };

        let result = match &effective_primary {
            RoutingTarget::Local { model: m } => {
                let r = self.local.chat(request, m, temperature).await;
                if should_fallback(&r, self.hints.privacy_required, &fallback) {
                    if let Some(RoutingTarget::Remote { model: fb }) = &fallback {
                        tracing::warn!(hint = model, "[routing] local chat fallback → remote");
                        fallback_occurred = true;
                        self.remote.chat(request, fb, temperature).await
                    } else {
                        r
                    }
                } else {
                    r
                }
            }
            RoutingTarget::Remote { model: m } => self.remote.chat(request, m, temperature).await,
        };

        let (input_tokens, output_tokens, cost_usd) = match &result {
            Ok(resp) => resp
                .usage
                .as_ref()
                .map(|u| (u.input_tokens, u.output_tokens, u.charged_amount_usd))
                .unwrap_or_default(),
            Err(_) => (0, 0, 0.0),
        };

        telemetry::emit(&RoutingRecord {
            model_hint: model.to_string(),
            task_category: category.as_str(),
            routed_to: if fallback_occurred {
                "remote"
            } else {
                effective_primary.label()
            },
            resolved_model: if fallback_occurred {
                fallback
                    .as_ref()
                    .map(|t| t.model().to_string())
                    .unwrap_or_default()
            } else {
                effective_primary.model().to_string()
            },
            local_healthy,
            fallback_to_remote: fallback_occurred,
            latency_ms: started.elapsed().as_millis() as u64,
            input_tokens,
            output_tokens,
            cost_usd,
        });

        result
    }
}

#[async_trait]
impl Provider for IntelligentRoutingProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        self.remote.capabilities()
    }

    fn convert_tools(&self, tools: &[ToolSpec]) -> ToolsPayload {
        self.remote.convert_tools(tools)
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> Result<String> {
        self.dispatch_chat_with_system(system_prompt, message, model, temperature)
            .await
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> Result<String> {
        let (primary, fallback, category, local_healthy) = self.resolve(model).await;
        let started = Instant::now();
        let mut fallback_occurred = false;

        let result = match &primary {
            RoutingTarget::Local { model: m } => {
                let r = self.local.chat_with_history(messages, m, temperature).await;
                let do_fallback = !self.hints.privacy_required
                    && fallback.is_some()
                    && match &r {
                        Err(_) => true,
                        Ok(text) => quality::is_low_quality(text),
                    };
                if do_fallback {
                    if let Some(RoutingTarget::Remote { model: fb }) = &fallback {
                        tracing::warn!(
                            hint = model,
                            "[routing] local history failed/low-quality → remote"
                        );
                        fallback_occurred = true;
                        self.remote
                            .chat_with_history(messages, fb, temperature)
                            .await
                    } else {
                        r
                    }
                } else {
                    r
                }
            }
            RoutingTarget::Remote { model: m } => {
                self.remote
                    .chat_with_history(messages, m, temperature)
                    .await
            }
        };

        telemetry::emit(&RoutingRecord {
            model_hint: model.to_string(),
            task_category: category.as_str(),
            routed_to: if fallback_occurred {
                "remote"
            } else {
                primary.label()
            },
            resolved_model: if fallback_occurred {
                fallback
                    .as_ref()
                    .map(|t| t.model().to_string())
                    .unwrap_or_default()
            } else {
                primary.model().to_string()
            },
            local_healthy,
            fallback_to_remote: fallback_occurred,
            latency_ms: started.elapsed().as_millis() as u64,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        });

        result
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> Result<ChatResponse> {
        self.dispatch_chat(request, model, temperature).await
    }

    fn supports_streaming(&self) -> bool {
        // With privacy_required we fail closed to local-only routing, and local
        // streaming is intentionally unsupported.
        !self.hints.privacy_required && self.remote.supports_streaming()
    }

    fn stream_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        options: StreamOptions,
    ) -> futures_util::stream::BoxStream<'static, StreamResult<StreamChunk>> {
        let (primary, remote_model) = self.resolve_streaming_target(model);

        match primary {
            RoutingTarget::Remote { .. } => self.remote.stream_chat_with_system(
                system_prompt,
                message,
                &remote_model,
                temperature,
                options,
            ),
            RoutingTarget::Local { .. } => {
                // Fail closed: do not bypass privacy/local routing by delegating
                // streaming to remote when policy chose local.
                Box::pin(futures_util::stream::once(async {
                    stream_local_not_supported_error()
                }))
            }
        }
    }

    async fn warmup(&self) -> Result<()> {
        self.remote.warmup().await?;
        if self.local_enabled {
            if let Err(e) = self.local.warmup().await {
                tracing::warn!(error = ?e, "[routing] local warmup failed (non-fatal)");
            }
        }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::providers::traits::ProviderCapabilities;
    use crate::openhuman::routing::health::LocalHealthChecker;
    use crate::openhuman::routing::policy::RoutingHints;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    // ── Mock provider ──────────────────────────────────────────────────────

    struct MockProvider {
        name: &'static str,
        calls: AtomicUsize,
        last_model: parking_lot::Mutex<String>,
        fail: AtomicBool,
        /// Fixed response text (controls quality check outcomes).
        response: parking_lot::Mutex<String>,
    }

    impl MockProvider {
        fn new(name: &'static str, response: &'static str) -> Arc<Self> {
            Arc::new(Self {
                name,
                calls: AtomicUsize::new(0),
                last_model: parking_lot::Mutex::new(String::new()),
                fail: AtomicBool::new(false),
                response: parking_lot::Mutex::new(response.to_string()),
            })
        }

        fn set_fail(&self, v: bool) {
            self.fail.store(v, Ordering::SeqCst);
        }

        fn set_response(&self, r: &str) {
            *self.response.lock() = r.to_string();
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn last_model(&self) -> String {
            self.last_model.lock().clone()
        }
    }

    #[async_trait]
    impl Provider for Arc<MockProvider> {
        async fn chat_with_system(
            &self,
            _system: Option<&str>,
            _msg: &str,
            model: &str,
            _temp: f64,
        ) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_model.lock() = model.to_string();
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("{} intentional failure", self.name);
            }
            Ok(self.response.lock().clone())
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                native_tool_calling: true,
                vision: false,
            }
        }
    }

    /// Build the routing provider with controllable health and hints.
    fn router(
        local: Arc<MockProvider>,
        remote: Arc<MockProvider>,
        health: Arc<LocalHealthChecker>,
        hints: RoutingHints,
    ) -> IntelligentRoutingProvider {
        IntelligentRoutingProvider::with_hints(
            Box::new(remote),
            Box::new(local),
            "gemma3:4b-it-qat".to_string(),
            "default-remote-model".to_string(),
            true,
            health,
            hints,
        )
    }

    // ── A. Local success path ──────────────────────────────────────────────

    #[tokio::test]
    async fn local_used_when_healthy_and_lightweight() {
        // Local is healthy → lightweight task must go to local.
        let local = MockProvider::new("local", "Great reaction!");
        let remote = MockProvider::new("remote", "remote-resp");
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        let result = r
            .chat_with_system(None, "React to this", "hint:reaction", 0.7)
            .await
            .unwrap();

        assert_eq!(result, "Great reaction!");
        assert_eq!(local.calls(), 1, "local must have been called");
        assert_eq!(remote.calls(), 0, "remote must NOT have been called");
        assert_eq!(local.last_model(), "gemma3:4b-it-qat");
    }

    #[tokio::test]
    async fn medium_without_hints_uses_remote() {
        let local = MockProvider::new("local", "Here is a summary.");
        let remote = MockProvider::new("remote", "remote-resp");
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        r.chat_with_system(None, "Summarize this", "hint:summarize", 0.7)
            .await
            .unwrap();

        assert_eq!(local.calls(), 0);
        assert_eq!(remote.calls(), 1);
    }

    #[tokio::test]
    async fn medium_with_local_bias_hint_uses_local() {
        let local = MockProvider::new("local", "Here is a local summary.");
        let remote = MockProvider::new("remote", "remote-resp");
        let health = LocalHealthChecker::seeded(true);
        let hints = RoutingHints {
            latency_budget: crate::openhuman::routing::policy::LatencyBudget::Low,
            ..Default::default()
        };

        let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
        r.chat_with_system(None, "Summarize this", "hint:summarize", 0.7)
            .await
            .unwrap();

        assert_eq!(local.calls(), 1);
        assert_eq!(remote.calls(), 0);
    }

    // ── B. Quality-based fallback ──────────────────────────────────────────

    #[tokio::test]
    async fn fallback_to_remote_when_local_response_low_quality() {
        let local = MockProvider::new("local", "I cannot help with that.");
        let remote = MockProvider::new("remote", "Actually here is a proper answer.");
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        let result = r
            .chat_with_system(None, "react", "hint:reaction", 0.7)
            .await
            .unwrap();

        // Local returns a refusal → quality fallback → remote answer
        assert_eq!(result, "Actually here is a proper answer.");
        assert_eq!(local.calls(), 1, "local tried first");
        assert_eq!(remote.calls(), 1, "remote called on quality fallback");
    }

    #[tokio::test]
    async fn fallback_to_remote_when_local_response_empty() {
        let local = MockProvider::new("local", "");
        let remote = MockProvider::new("remote", "Good answer from remote.");
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        let result = r
            .chat_with_system(None, "classify", "hint:classify", 0.7)
            .await
            .unwrap();

        assert_eq!(result, "Good answer from remote.");
        assert_eq!(remote.calls(), 1);
    }

    // ── C. Error-based fallback ────────────────────────────────────────────

    #[tokio::test]
    async fn fallback_to_remote_when_local_errors() {
        let local = MockProvider::new("local", "never returned");
        local.set_fail(true);
        let remote = MockProvider::new("remote", "remote recovered");
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        let result = r
            .chat_with_system(None, "react", "hint:reaction", 0.7)
            .await
            .unwrap();

        assert_eq!(result, "remote recovered");
        assert_eq!(local.calls(), 1);
        assert_eq!(remote.calls(), 1);
    }

    // ── D. Remote-only when local unhealthy ───────────────────────────────

    #[tokio::test]
    async fn remote_when_local_unhealthy() {
        let local = MockProvider::new("local", "never used");
        let remote = MockProvider::new("remote", "remote answer");
        let health = LocalHealthChecker::seeded(false);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        r.chat_with_system(None, "react", "hint:reaction", 0.7)
            .await
            .unwrap();

        assert_eq!(local.calls(), 0, "local must not be called when unhealthy");
        assert_eq!(remote.calls(), 1);
    }

    // ── E. Heavy tasks always remote ──────────────────────────────────────

    #[tokio::test]
    async fn heavy_tasks_always_use_remote() {
        let local = MockProvider::new("local", "should not be called");
        let remote = MockProvider::new("remote", "reasoning answer");
        let health = LocalHealthChecker::seeded(true); // local is healthy

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        r.chat_with_system(None, "reason hard", "hint:reasoning", 0.7)
            .await
            .unwrap();

        assert_eq!(local.calls(), 0, "heavy tasks must never use local");
        assert_eq!(remote.calls(), 1);
        assert_eq!(remote.last_model(), "reasoning-v1");
    }

    // ── F. Privacy override ────────────────────────────────────────────────

    #[tokio::test]
    async fn privacy_required_never_falls_back_to_remote() {
        let local = MockProvider::new("local", "I cannot help with that.");
        local.set_fail(false); // returns low-quality, not an error
        let remote = MockProvider::new("remote", "would breach privacy");
        let health = LocalHealthChecker::seeded(true);
        let hints = RoutingHints {
            privacy_required: true,
            ..Default::default()
        };

        let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
        // Local returns a refusal (low quality) but privacy blocks fallback.
        let result = r
            .chat_with_system(None, "private data", "hint:reaction", 0.7)
            .await
            .unwrap();

        assert!(result.contains("cannot"), "got: {result}");
        assert_eq!(
            remote.calls(),
            0,
            "remote must never be called with privacy_required"
        );
    }

    #[tokio::test]
    async fn privacy_required_even_for_heavy_tasks() {
        // Heavy + privacy_required → still local, no remote
        let local = MockProvider::new("local", "local heavy response");
        let remote = MockProvider::new("remote", "remote");
        let health = LocalHealthChecker::seeded(true);
        let hints = RoutingHints {
            privacy_required: true,
            ..Default::default()
        };

        let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
        r.chat_with_system(None, "reason", "hint:reasoning", 0.7)
            .await
            .unwrap();

        assert_eq!(local.calls(), 1);
        assert_eq!(remote.calls(), 0);
    }

    // ── G. Latency / cost hints ────────────────────────────────────────────

    #[tokio::test]
    async fn low_latency_hint_prefers_local() {
        let local = MockProvider::new("local", "fast local answer");
        let remote = MockProvider::new("remote", "slower remote");
        let health = LocalHealthChecker::seeded(true);
        let hints = RoutingHints {
            latency_budget: crate::openhuman::routing::policy::LatencyBudget::Low,
            ..Default::default()
        };

        let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
        r.chat_with_system(None, "quick task", "hint:reaction", 0.7)
            .await
            .unwrap();

        assert_eq!(local.calls(), 1);
        assert_eq!(remote.calls(), 0);
    }

    // ── H. Integration: local disabled ────────────────────────────────────

    #[tokio::test]
    async fn local_disabled_all_tasks_go_remote() {
        let local = MockProvider::new("local", "should not be called");
        let remote = MockProvider::new("remote", "remote answer");
        let health = LocalHealthChecker::seeded(true);

        // Build with local_enabled = false
        let r = IntelligentRoutingProvider::new(
            Box::new(Arc::clone(&remote)),
            Box::new(Arc::clone(&local)),
            "local-model".to_string(),
            "default-remote-model".to_string(),
            false, // disabled
            health,
        );
        r.chat_with_system(None, "react", "hint:reaction", 0.7)
            .await
            .unwrap();

        assert_eq!(local.calls(), 0);
        assert_eq!(remote.calls(), 1);
    }

    // ── I. Regression ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn regression_reasoning_hint_routes_remote_with_backend_model_name() {
        let local = MockProvider::new("local", "l");
        let remote = MockProvider::new("remote", "r");
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        r.chat_with_system(None, "reason", "hint:reasoning", 0.7)
            .await
            .unwrap();

        // Heavy reasoning hints must be normalized to backend-valid model IDs.
        assert_eq!(remote.last_model(), "reasoning-v1");
        assert_eq!(local.calls(), 0);
    }

    #[tokio::test]
    async fn remote_failure_propagates_without_local_fallback() {
        let local = MockProvider::new("local", "l");
        let remote = MockProvider::new("remote", "r");
        remote.set_fail(true);
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        // Heavy task goes remote, remote fails → error propagates, no local retry.
        let err = r
            .chat_with_system(None, "reason", "hint:reasoning", 0.7)
            .await;
        assert!(err.is_err());
        assert_eq!(local.calls(), 0);
    }

    #[tokio::test]
    async fn warmup_remote_failure_is_fatal_local_is_not() {
        let local = MockProvider::new("local", "l");
        local.set_fail(true);
        let remote = MockProvider::new("remote", "r");
        let health = LocalHealthChecker::seeded(true);

        let r = router(
            Arc::clone(&local),
            Arc::clone(&remote),
            health,
            RoutingHints::default(),
        );
        assert!(
            r.warmup().await.is_ok(),
            "local warmup failure must not propagate"
        );
    }

    #[tokio::test]
    async fn capabilities_delegate_to_remote() {
        let local = MockProvider::new("local", "l");
        let remote = MockProvider::new("remote", "r");
        let health = LocalHealthChecker::seeded(true);
        let r = router(local, remote, health, RoutingHints::default());
        assert!(r.capabilities().native_tool_calling);
    }
}
