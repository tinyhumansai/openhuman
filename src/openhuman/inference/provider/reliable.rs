//! # Deletion staged (issue #4249, Workstream 02.2)
//!
//! `ReliableProvider` is **slated for removal** once the tinyagents crate owns
//! retry/fallback with proven parity. As of 02.2 the crate path now covers this
//! module's responsibilities: transient-vs-permanent error classification (the
//! `is_non_retryable` / `is_rate_limited` / `is_upstream_unhealthy` helpers here
//! are reused by `tinyagents::model::ProviderModel` to map errors onto the crate's
//! retryable/non-retryable `TinyAgentsError` variants), exponential-backoff retry
//! (`RunPolicy.retry`, pinned to a single attempt until this wrapper is removed),
//! and cross-route model fallover (`RunPolicy.fallback` + the event-visible
//! `FallbackObserverMiddleware`, which additionally fails over across the
//! registered workload-tier routes — something this wrapper never did).
//!
//! **Do not delete yet.** Removal is gated on the deferred conformance pass
//! (Workstream 11): un-wrapping `ReliableProvider` from its remaining call sites
//! (session builder/factory + the non-turn callers: memory-tree local summarizer,
//! memory scoring, triage classification), flipping `RunPolicy.retry.max_attempts`
//! off the single-attempt pin, and rewriting the behaviorally-relevant tests here
//! (attempt-count parity for 429 / 500 / config-rejection / billing, retry/fallback
//! event visibility, and no-double-retry) against the crate loop. Until then this
//! wrapper stays authoritative for single-attempt retry on the live path.

use super::traits::{
    ChatMessage, ChatRequest, ChatResponse, StreamChunk, StreamError, StreamOptions, StreamResult,
};
use super::Provider;
use crate::openhuman::inference::provider::record_resolved_provider_route;
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

// The pure, stateless error-classification / backoff helpers now live in the
// sibling `error_classify` module (issue #4249, Workstream 02.2). Re-export
// them at crate visibility so `ReliableProvider` below keeps using them
// unchanged AND existing external `reliable::is_non_retryable` /
// `reliable::{is_rate_limited, is_upstream_unhealthy, parse_retry_after_ms}` /
// `reliable::format_failure_aggregate` import paths continue to resolve.
pub(crate) use super::error_classify::*;

fn push_failure(
    failures: &mut Vec<String>,
    provider_name: &str,
    model: &str,
    attempt: u32,
    max_attempts: u32,
    reason: &str,
    error_detail: &str,
) {
    failures.push(format!(
        "provider={provider_name} model={model} attempt {attempt}/{max_attempts}: {reason}; error={error_detail}"
    ));
}

/// Provider wrapper with retry, fallback, auth rotation, and model failover.
pub struct ReliableProvider {
    /// Stored behind `Arc` (not `Box`) so the streaming failover path can hand
    /// owned, `'static` provider handles to the consumer task and create
    /// candidate streams *lazily* — issuing each upstream request only when the
    /// previous candidate has actually failed (see `stream_chat_with_system`).
    /// The public `new` constructor still accepts `Box<dyn Provider>`; the
    /// conversion happens internally so callers are unaffected.
    providers: Vec<(String, std::sync::Arc<dyn Provider>)>,
    max_retries: u32,
    base_backoff_ms: u64,
    /// Extra API keys for rotation (index tracks round-robin position).
    api_keys: Vec<String>,
    key_index: AtomicUsize,
    /// Per-model fallback chains: model_name → [fallback_model_1, fallback_model_2, ...]
    model_fallbacks: HashMap<String, Vec<String>>,
}

impl ReliableProvider {
    pub fn new(
        providers: Vec<(String, Box<dyn Provider>)>,
        max_retries: u32,
        base_backoff_ms: u64,
    ) -> Self {
        Self {
            providers: providers
                .into_iter()
                .map(|(name, p)| (name, std::sync::Arc::from(p)))
                .collect(),
            max_retries,
            base_backoff_ms: base_backoff_ms.max(50),
            api_keys: Vec::new(),
            key_index: AtomicUsize::new(0),
            model_fallbacks: HashMap::new(),
        }
    }

    /// Set additional API keys for round-robin rotation on rate-limit errors.
    pub fn with_api_keys(mut self, keys: Vec<String>) -> Self {
        self.api_keys = keys;
        self
    }

    /// Set per-model fallback chains.
    pub fn with_model_fallbacks(mut self, fallbacks: HashMap<String, Vec<String>>) -> Self {
        self.model_fallbacks = fallbacks;
        self
    }

    /// Build the list of models to try: [original, fallback1, fallback2, ...]
    fn model_chain<'a>(&'a self, model: &'a str) -> Vec<&'a str> {
        let mut chain = vec![model];
        if let Some(fallbacks) = self.model_fallbacks.get(model) {
            chain.extend(fallbacks.iter().map(|s| s.as_str()));
        }
        chain
    }

    /// Advance to the next API key and return it, or None if no extra keys configured.
    fn rotate_key(&self) -> Option<&str> {
        if self.api_keys.is_empty() {
            return None;
        }
        let idx = self.key_index.fetch_add(1, Ordering::Relaxed) % self.api_keys.len();
        Some(&self.api_keys[idx])
    }

    /// Compute backoff duration, respecting Retry-After if present.
    fn compute_backoff(&self, base: u64, err: &anyhow::Error) -> u64 {
        if let Some(retry_after) = parse_retry_after_ms(err) {
            // Use Retry-After but cap at 30s to avoid indefinite waits
            retry_after.min(30_000).max(base)
        } else {
            base
        }
    }
}

#[async_trait]
impl Provider for ReliableProvider {
    fn telemetry_provider_id(&self) -> String {
        // Delegate to the primary (first) upstream - the one that serves the
        // call unless a failover kicks in.
        self.providers
            .first()
            .map(|(_, p)| p.telemetry_provider_id())
            .unwrap_or_else(|| "custom".to_string())
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        for (name, provider) in &self.providers {
            tracing::info!(provider = name, "Warming up provider connection pool");
            if provider.warmup().await.is_err() {
                tracing::warn!(provider = name, "Warmup failed (non-fatal)");
            }
        }
        Ok(())
    }

    /// Delegate to the primary provider so a wrapped local runtime reports its
    /// runtime-loaded window (LM Studio `n_ctx`) for pre-dispatch trimming
    /// instead of the static-table default (#3550 / TAURI-RUST-6V0).
    async fn effective_context_window(&self, model: &str) -> Option<u64> {
        match self.providers.first() {
            Some((_, provider)) => provider.effective_context_window(model).await,
            None => crate::openhuman::inference::context_window_for_model(model),
        }
    }

    /// Delegate to the primary provider so the engine's pre-dispatch
    /// un-evictable-prefix guard fires for a wrapped local model (#3550).
    fn is_local_provider(&self) -> bool {
        self.providers
            .first()
            .map(|(_, p)| p.is_local_provider())
            .unwrap_or(false)
    }

    /// Delegate the model-aware locality to the primary provider so a wrapped
    /// router resolves `model` to its actual (possibly local) provider for the
    /// engine's pre-dispatch guard (#3550 / PR #3771).
    fn is_local_provider_for_model(&self, model: &str) -> bool {
        self.providers
            .first()
            .map(|(_, p)| p.is_local_provider_for_model(model))
            .unwrap_or(false)
    }

    /// Delegate the authoritative runtime-loaded window to the primary provider
    /// so the engine's hard pre-dispatch abort sees the wrapped local runtime's
    /// loaded `n_ctx` (#3550 / PR #3771).
    async fn loaded_context_window(&self, model: &str) -> Option<u64> {
        match self.providers.first() {
            Some((_, provider)) => provider.loaded_context_window(model).await,
            None => None,
        }
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let models = self.model_chain(model);
        let mut failures = Vec::new();

        for current_model in &models {
            for (provider_name, provider) in &self.providers {
                let mut backoff_ms = self.base_backoff_ms;

                for attempt in 0..=self.max_retries {
                    record_resolved_provider_route(provider_name, *current_model);
                    match provider
                        .chat_with_system(system_prompt, message, current_model, temperature)
                        .await
                    {
                        Ok(resp) => {
                            if attempt > 0 || *current_model != model {
                                tracing::info!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt,
                                    original_model = model,
                                    "Provider recovered (failover/retry)"
                                );
                            }
                            return Ok(resp);
                        }
                        Err(e) => {
                            let non_retryable_rate_limit = is_non_retryable_rate_limit(&e);
                            let non_retryable = is_non_retryable(&e) || non_retryable_rate_limit;
                            let rate_limited = is_rate_limited(&e);
                            let upstream_unhealthy = is_upstream_unhealthy(&e);
                            let failure_reason =
                                failure_reason(rate_limited, non_retryable, upstream_unhealthy);
                            let error_detail = compact_error_detail(&e);

                            push_failure(
                                &mut failures,
                                provider_name,
                                current_model,
                                attempt + 1,
                                self.max_retries + 1,
                                failure_reason,
                                &error_detail,
                            );

                            // On rate-limit, try rotating API key
                            if rate_limited && !non_retryable_rate_limit {
                                if self.rotate_key().is_some() {
                                    tracing::info!(
                                        provider = provider_name,
                                        error = %error_detail,
                                        key_slot = %rotated_key_log_detail(
                                            self.key_index.load(Ordering::Relaxed),
                                            self.api_keys.len()
                                        ),
                                        "Rate limited, rotated API key"
                                    );
                                }
                            }

                            if non_retryable {
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    error = %error_detail,
                                    "Non-retryable error, moving on"
                                );

                                if is_context_window_exceeded(&e) {
                                    anyhow::bail!(
                                        "Request exceeds model context window; retries and fallbacks were skipped. Attempts:\n{}",
                                        failures.join("\n")
                                    );
                                }

                                break;
                            }

                            if attempt < self.max_retries {
                                let wait = self.compute_backoff(backoff_ms, &e);
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt = attempt + 1,
                                    backoff_ms = wait,
                                    reason = failure_reason,
                                    error = %error_detail,
                                    "Provider call failed, retrying"
                                );
                                tokio::time::sleep(Duration::from_millis(wait)).await;
                                backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            }
                        }
                    }
                }

                tracing::warn!(
                    provider = provider_name,
                    model = *current_model,
                    "Exhausted retries, trying next provider/model"
                );
            }

            if *current_model != model {
                tracing::warn!(
                    original_model = model,
                    fallback_model = *current_model,
                    "Model fallback exhausted all providers, trying next fallback model"
                );
            }
        }

        let aggregate = format_failure_aggregate(
            model,
            &failures,
            self.model_fallbacks
                .get(model)
                .is_some_and(|chain| !chain.is_empty()),
        );
        crate::core::observability::report_error_or_expected(
            aggregate.as_str(),
            "llm_provider",
            "reliable_chat_with_system",
            &[
                ("model", model),
                ("attempts", &failures.len().to_string()),
                ("failure", "all_exhausted"),
            ],
        );
        anyhow::bail!(aggregate)
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let models = self.model_chain(model);
        let mut failures = Vec::new();

        for current_model in &models {
            for (provider_name, provider) in &self.providers {
                let mut backoff_ms = self.base_backoff_ms;

                for attempt in 0..=self.max_retries {
                    record_resolved_provider_route(provider_name, *current_model);
                    match provider
                        .chat_with_history(messages, current_model, temperature)
                        .await
                    {
                        Ok(resp) => {
                            if attempt > 0 || *current_model != model {
                                tracing::info!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt,
                                    original_model = model,
                                    "Provider recovered (failover/retry)"
                                );
                            }
                            return Ok(resp);
                        }
                        Err(e) => {
                            let non_retryable_rate_limit = is_non_retryable_rate_limit(&e);
                            let non_retryable = is_non_retryable(&e) || non_retryable_rate_limit;
                            let rate_limited = is_rate_limited(&e);
                            let upstream_unhealthy = is_upstream_unhealthy(&e);
                            let failure_reason =
                                failure_reason(rate_limited, non_retryable, upstream_unhealthy);
                            let error_detail = compact_error_detail(&e);

                            push_failure(
                                &mut failures,
                                provider_name,
                                current_model,
                                attempt + 1,
                                self.max_retries + 1,
                                failure_reason,
                                &error_detail,
                            );

                            if rate_limited && !non_retryable_rate_limit {
                                if self.rotate_key().is_some() {
                                    tracing::info!(
                                        provider = provider_name,
                                        error = %error_detail,
                                        key_slot = %rotated_key_log_detail(
                                            self.key_index.load(Ordering::Relaxed),
                                            self.api_keys.len()
                                        ),
                                        "Rate limited, rotated API key"
                                    );
                                }
                            }

                            if non_retryable {
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    error = %error_detail,
                                    "Non-retryable error, moving on"
                                );

                                if is_context_window_exceeded(&e) {
                                    anyhow::bail!(
                                        "Request exceeds model context window; retries and fallbacks were skipped. Attempts:\n{}",
                                        failures.join("\n")
                                    );
                                }

                                break;
                            }

                            if attempt < self.max_retries {
                                let wait = self.compute_backoff(backoff_ms, &e);
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt = attempt + 1,
                                    backoff_ms = wait,
                                    reason = failure_reason,
                                    error = %error_detail,
                                    "Provider call failed, retrying"
                                );
                                tokio::time::sleep(Duration::from_millis(wait)).await;
                                backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            }
                        }
                    }
                }

                tracing::warn!(
                    provider = provider_name,
                    model = *current_model,
                    "Exhausted retries, trying next provider/model"
                );
            }
        }

        let aggregate = format_failure_aggregate(
            model,
            &failures,
            self.model_fallbacks
                .get(model)
                .is_some_and(|chain| !chain.is_empty()),
        );
        crate::core::observability::report_error_or_expected(
            aggregate.as_str(),
            "llm_provider",
            "reliable_chat_with_history",
            &[
                ("model", model),
                ("attempts", &failures.len().to_string()),
                ("failure", "all_exhausted"),
            ],
        );
        anyhow::bail!(aggregate)
    }

    fn supports_native_tools(&self) -> bool {
        self.providers
            .first()
            .map(|(_, p)| p.supports_native_tools())
            .unwrap_or(false)
    }

    fn supports_vision(&self) -> bool {
        self.providers
            .iter()
            .any(|(_, provider)| provider.supports_vision())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let models = self.model_chain(model);
        let mut failures = Vec::new();

        for current_model in &models {
            for (provider_name, provider) in &self.providers {
                let mut backoff_ms = self.base_backoff_ms;

                for attempt in 0..=self.max_retries {
                    // Only forward the streaming sender on the first
                    // attempt. A failed attempt that partially streamed
                    // text/args has already published those fragments to
                    // the downstream progress bridge; if a retry also
                    // streamed, the consumer would see duplicated tokens
                    // and mismatched tool_call_ids. Retries silently
                    // degrade to non-streaming and the caller still gets
                    // a correct aggregated response from `chat()`.
                    let stream_this_attempt = if attempt == 0 {
                        request.stream
                    } else {
                        if request.stream.is_some() {
                            tracing::info!(
                                provider = provider_name,
                                model = *current_model,
                                attempt,
                                "[reliable] retry forcing non-streaming to avoid duplicate deltas"
                            );
                        }
                        None
                    };
                    let req = ChatRequest {
                        messages: request.messages,
                        tools: request.tools,
                        stream: stream_this_attempt,
                        max_tokens: request.max_tokens,
                    };
                    record_resolved_provider_route(provider_name, *current_model);
                    match provider.chat(req, current_model, temperature).await {
                        Ok(resp) => {
                            if attempt > 0 || *current_model != model {
                                tracing::info!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt,
                                    original_model = model,
                                    "Provider recovered (failover/retry)"
                                );
                            }
                            return Ok(resp);
                        }
                        Err(e) => {
                            let non_retryable_rate_limit = is_non_retryable_rate_limit(&e);
                            let non_retryable = is_non_retryable(&e) || non_retryable_rate_limit;
                            let rate_limited = is_rate_limited(&e);
                            let upstream_unhealthy = is_upstream_unhealthy(&e);
                            let failure_reason =
                                failure_reason(rate_limited, non_retryable, upstream_unhealthy);
                            let error_detail = compact_error_detail(&e);

                            push_failure(
                                &mut failures,
                                provider_name,
                                current_model,
                                attempt + 1,
                                self.max_retries + 1,
                                failure_reason,
                                &error_detail,
                            );

                            if rate_limited && !non_retryable_rate_limit {
                                if self.rotate_key().is_some() {
                                    tracing::info!(
                                        provider = provider_name,
                                        error = %error_detail,
                                        key_slot = %rotated_key_log_detail(
                                            self.key_index.load(Ordering::Relaxed),
                                            self.api_keys.len()
                                        ),
                                        "Rate limited, rotated API key"
                                    );
                                }
                            }

                            if non_retryable {
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    error = %error_detail,
                                    "Non-retryable error, moving on"
                                );

                                if is_context_window_exceeded(&e) {
                                    anyhow::bail!(
                                        "Request exceeds model context window; retries and fallbacks were skipped. Attempts:\n{}",
                                        failures.join("\n")
                                    );
                                }

                                break;
                            }

                            if attempt < self.max_retries {
                                let wait = self.compute_backoff(backoff_ms, &e);
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt = attempt + 1,
                                    backoff_ms = wait,
                                    reason = failure_reason,
                                    error = %error_detail,
                                    "Provider call failed, retrying"
                                );
                                tokio::time::sleep(Duration::from_millis(wait)).await;
                                backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            }
                        }
                    }
                }

                tracing::warn!(
                    provider = provider_name,
                    model = *current_model,
                    "Exhausted retries, trying next provider/model"
                );
            }
        }

        let aggregate = format_failure_aggregate(
            model,
            &failures,
            self.model_fallbacks
                .get(model)
                .is_some_and(|chain| !chain.is_empty()),
        );
        crate::core::observability::report_error_or_expected(
            aggregate.as_str(),
            "llm_provider",
            "reliable_chat",
            &[
                ("model", model),
                ("attempts", &failures.len().to_string()),
                ("failure", "all_exhausted"),
            ],
        );
        anyhow::bail!(aggregate)
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let models = self.model_chain(model);
        let mut failures = Vec::new();

        for current_model in &models {
            for (provider_name, provider) in &self.providers {
                let mut backoff_ms = self.base_backoff_ms;

                for attempt in 0..=self.max_retries {
                    record_resolved_provider_route(provider_name, *current_model);
                    match provider
                        .chat_with_tools(messages, tools, current_model, temperature)
                        .await
                    {
                        Ok(resp) => {
                            if attempt > 0 || *current_model != model {
                                tracing::info!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt,
                                    original_model = model,
                                    "Provider recovered (failover/retry)"
                                );
                            }
                            return Ok(resp);
                        }
                        Err(e) => {
                            let non_retryable_rate_limit = is_non_retryable_rate_limit(&e);
                            let non_retryable = is_non_retryable(&e) || non_retryable_rate_limit;
                            let rate_limited = is_rate_limited(&e);
                            let upstream_unhealthy = is_upstream_unhealthy(&e);
                            let failure_reason =
                                failure_reason(rate_limited, non_retryable, upstream_unhealthy);
                            let error_detail = compact_error_detail(&e);

                            push_failure(
                                &mut failures,
                                provider_name,
                                current_model,
                                attempt + 1,
                                self.max_retries + 1,
                                failure_reason,
                                &error_detail,
                            );

                            if rate_limited && !non_retryable_rate_limit {
                                if self.rotate_key().is_some() {
                                    tracing::info!(
                                        provider = provider_name,
                                        error = %error_detail,
                                        key_slot = %rotated_key_log_detail(
                                            self.key_index.load(Ordering::Relaxed),
                                            self.api_keys.len()
                                        ),
                                        "Rate limited, rotated API key"
                                    );
                                }
                            }

                            if non_retryable {
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    error = %error_detail,
                                    "Non-retryable error, moving on"
                                );

                                if is_context_window_exceeded(&e) {
                                    anyhow::bail!(
                                        "Request exceeds model context window; retries and fallbacks were skipped. Attempts:\n{}",
                                        failures.join("\n")
                                    );
                                }

                                break;
                            }

                            if attempt < self.max_retries {
                                let wait = self.compute_backoff(backoff_ms, &e);
                                tracing::warn!(
                                    provider = provider_name,
                                    model = *current_model,
                                    attempt = attempt + 1,
                                    backoff_ms = wait,
                                    reason = failure_reason,
                                    error = %error_detail,
                                    "Provider call failed, retrying"
                                );
                                tokio::time::sleep(Duration::from_millis(wait)).await;
                                backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            }
                        }
                    }
                }

                tracing::warn!(
                    provider = provider_name,
                    model = *current_model,
                    "Exhausted retries, trying next provider/model"
                );
            }
        }

        let aggregate = format_failure_aggregate(
            model,
            &failures,
            self.model_fallbacks
                .get(model)
                .is_some_and(|chain| !chain.is_empty()),
        );
        crate::core::observability::report_error_or_expected(
            aggregate.as_str(),
            "llm_provider",
            "reliable_chat_with_tools",
            &[
                ("model", model),
                ("attempts", &failures.len().to_string()),
                ("failure", "all_exhausted"),
            ],
        );
        anyhow::bail!(aggregate)
    }

    fn supports_streaming(&self) -> bool {
        self.providers.iter().any(|(_, p)| p.supports_streaming())
    }

    fn stream_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        if !options.enabled {
            return stream::once(async move {
                Err(super::traits::StreamError::Provider(
                    "Streaming disabled".to_string(),
                ))
            })
            .boxed();
        }

        // Collect streaming-capable providers
        let streaming_providers: Vec<_> = self
            .providers
            .iter()
            .filter(|(_, p)| p.supports_streaming())
            .collect();

        if streaming_providers.is_empty() {
            return stream::once(async move {
                Err(super::traits::StreamError::Provider(
                    "No provider supports streaming".to_string(),
                ))
            })
            .boxed();
        }

        // Build model chain and provider info for the spawned task
        let models = self.model_chain(model);
        let model_chain: Vec<String> = models.into_iter().map(|m| m.to_string()).collect();
        let base_backoff_ms = self.base_backoff_ms;

        // Capture only owned `(provider_name, provider, model)` *tuples* up-front
        // — NOT the streams themselves. The provider impl spawns the upstream
        // HTTP POST the instant a stream is created, so eagerly building the
        // full provider×model product here would fire every fallback request
        // at once (duplicate billing/side-effects). Instead we clone the
        // `Arc<dyn Provider>` handles and call `stream_chat_with_system` lazily
        // inside the consumer task, immediately before each candidate is tried
        // — mirroring the sequential non-streaming paths. (audit C2)
        //
        // `system_prompt` / `message` are borrowed from the caller and the
        // spawned task is `'static`, so own them here.
        let system_prompt_owned: Option<String> = system_prompt.map(|s| s.to_string());
        let message_owned: String = message.to_string();
        let mut candidates: Vec<(String, std::sync::Arc<dyn Provider>, String)> = Vec::new();
        for current_model in &model_chain {
            for (provider_name, provider) in &streaming_providers {
                candidates.push((
                    (*provider_name).clone(),
                    std::sync::Arc::clone(provider),
                    current_model.clone(),
                ));
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);
        let max_retries = self.max_retries;

        tokio::spawn(async move {
            for (provider_name, provider, current_model) in candidates {
                let mut backoff_ms = base_backoff_ms;
                let mut attempts = 0u32;

                loop {
                    // Create (and thereby fire) the candidate stream lazily here,
                    // immediately before we attempt it. On a retryable failure we
                    // re-create it on the next loop iteration rather than
                    // re-polling the previous, already-exhausted stream (which
                    // only yields `None` after its single error). (audit C2/C6)
                    let mut candidate_stream = provider.stream_chat_with_system(
                        system_prompt_owned.as_deref(),
                        &message_owned,
                        &current_model,
                        temperature,
                        options,
                    );

                    match candidate_stream.next().await {
                        Some(Ok(chunk)) => {
                            // First chunk succeeded — commit to this stream
                            if tx.send(Ok(chunk)).await.is_err() {
                                return;
                            }
                            // Forward remaining chunks
                            while let Some(chunk) = candidate_stream.next().await {
                                if tx.send(chunk).await.is_err() {
                                    return;
                                }
                            }
                            return; // Done successfully
                        }
                        Some(Err(ref e)) => {
                            let non_retryable = is_stream_error_non_retryable(e);

                            tracing::warn!(
                                provider = provider_name,
                                model = current_model,
                                attempt = attempts + 1,
                                error = %e,
                                "Streaming failed{}", if non_retryable { " (non-retryable)" } else { "" }
                            );

                            if non_retryable || attempts >= max_retries {
                                break; // Move to next candidate
                            }

                            attempts += 1;
                            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                            backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                            // Re-create the candidate stream on the next iteration.
                            continue;
                        }
                        None => {
                            // Stream exhausted without success
                            if attempts == 0 {
                                tracing::warn!(
                                    provider = provider_name,
                                    model = current_model,
                                    "Stream returned empty"
                                );
                            }
                            break; // Move to next candidate
                        }
                    }
                }
            }

            // All providers/models exhausted
            let _ = tx
                .send(Err(super::traits::StreamError::Provider(
                    "All streaming providers/models failed".to_string(),
                )))
                .await;
        });

        stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed()
    }
}

#[cfg(test)]
#[path = "reliable_tests.rs"]
mod tests;
