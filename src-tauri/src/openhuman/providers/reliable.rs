use super::traits::{ChatMessage, ChatResponse, StreamChunk, StreamOptions, StreamResult};
use super::Provider;
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Check if an error is non-retryable (client errors that won't resolve with retries).
fn is_non_retryable(err: &anyhow::Error) -> bool {
    if is_context_window_exceeded(err) {
        return true;
    }

    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            let code = status.as_u16();
            return status.is_client_error() && code != 429 && code != 408;
        }
    }
    let msg = err.to_string();
    for word in msg.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = word.parse::<u16>() {
            if (400..500).contains(&code) {
                return code != 429 && code != 408;
            }
        }
    }

    let msg_lower = msg.to_lowercase();
    let auth_failure_hints = [
        "invalid api key",
        "incorrect api key",
        "missing api key",
        "api key not set",
        "authentication failed",
        "auth failed",
        "unauthorized",
        "forbidden",
        "permission denied",
        "access denied",
        "invalid token",
    ];

    if auth_failure_hints
        .iter()
        .any(|hint| msg_lower.contains(hint))
    {
        return true;
    }

    msg_lower.contains("model")
        && (msg_lower.contains("not found")
            || msg_lower.contains("unknown")
            || msg_lower.contains("unsupported")
            || msg_lower.contains("does not exist")
            || msg_lower.contains("invalid"))
}

fn is_context_window_exceeded(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    let hints = [
        "exceeds the context window",
        "context window of this model",
        "maximum context length",
        "context length exceeded",
        "too many tokens",
        "token limit exceeded",
        "prompt is too long",
        "input is too long",
    ];

    hints.iter().any(|hint| lower.contains(hint))
}

/// Check if an error is a rate-limit (429) error.
fn is_rate_limited(err: &anyhow::Error) -> bool {
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            return status.as_u16() == 429;
        }
    }
    let msg = err.to_string();
    msg.contains("429")
        && (msg.contains("Too Many") || msg.contains("rate") || msg.contains("limit"))
}

/// Check if a 429 is a business/quota-plan error that retries cannot fix.
///
/// Examples:
/// - plan does not include requested model
/// - insufficient balance / package not active
/// - known provider business codes (e.g. Z.AI: 1311, 1113)
fn is_non_retryable_rate_limit(err: &anyhow::Error) -> bool {
    if !is_rate_limited(err) {
        return false;
    }

    let msg = err.to_string();
    let lower = msg.to_lowercase();

    let business_hints = [
        "plan does not include",
        "doesn't include",
        "not include",
        "insufficient balance",
        "insufficient_balance",
        "insufficient quota",
        "insufficient_quota",
        "quota exhausted",
        "out of credits",
        "no available package",
        "package not active",
        "purchase package",
        "model not available for your plan",
    ];

    if business_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    // Known provider business codes observed for 429 where retry is futile.
    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if matches!(code, 1113 | 1311) {
                return true;
            }
        }
    }

    false
}

/// Try to extract a Retry-After value (in milliseconds) from an error message.
/// Looks for patterns like `Retry-After: 5` or `retry_after: 2.5` in the error string.
fn parse_retry_after_ms(err: &anyhow::Error) -> Option<u64> {
    let msg = err.to_string();
    let lower = msg.to_lowercase();

    // Look for "retry-after: <number>" or "retry_after: <number>"
    for prefix in &[
        "retry-after:",
        "retry_after:",
        "retry-after ",
        "retry_after ",
    ] {
        if let Some(pos) = lower.find(prefix) {
            let after = &msg[pos + prefix.len()..];
            let num_str: String = after
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(secs) = num_str.parse::<f64>() {
                if secs.is_finite() && secs >= 0.0 {
                    let millis = Duration::from_secs_f64(secs).as_millis();
                    if let Ok(value) = u64::try_from(millis) {
                        return Some(value);
                    }
                }
            }
        }
    }
    None
}

fn failure_reason(rate_limited: bool, non_retryable: bool) -> &'static str {
    if rate_limited && non_retryable {
        "rate_limited_non_retryable"
    } else if rate_limited {
        "rate_limited"
    } else if non_retryable {
        "non_retryable"
    } else {
        "retryable"
    }
}

fn compact_error_detail(err: &anyhow::Error) -> String {
    super::sanitize_api_error(&err.to_string())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

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
    providers: Vec<(String, Box<dyn Provider>)>,
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
            providers,
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
    async fn warmup(&self) -> anyhow::Result<()> {
        for (name, provider) in &self.providers {
            tracing::info!(provider = name, "Warming up provider connection pool");
            if provider.warmup().await.is_err() {
                tracing::warn!(provider = name, "Warmup failed (non-fatal)");
            }
        }
        Ok(())
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
                            let failure_reason = failure_reason(rate_limited, non_retryable);
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
                                if let Some(new_key) = self.rotate_key() {
                                    tracing::info!(
                                        provider = provider_name,
                                        error = %error_detail,
                                        "Rate limited, rotated API key (key ending ...{})",
                                        &new_key[new_key.len().saturating_sub(4)..]
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

        anyhow::bail!(
            "All providers/models failed. Attempts:\n{}",
            failures.join("\n")
        )
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
                            let failure_reason = failure_reason(rate_limited, non_retryable);
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
                                if let Some(new_key) = self.rotate_key() {
                                    tracing::info!(
                                        provider = provider_name,
                                        error = %error_detail,
                                        "Rate limited, rotated API key (key ending ...{})",
                                        &new_key[new_key.len().saturating_sub(4)..]
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

        anyhow::bail!(
            "All providers/models failed. Attempts:\n{}",
            failures.join("\n")
        )
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
                            let failure_reason = failure_reason(rate_limited, non_retryable);
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
                                if let Some(new_key) = self.rotate_key() {
                                    tracing::info!(
                                        provider = provider_name,
                                        error = %error_detail,
                                        "Rate limited, rotated API key (key ending ...{})",
                                        &new_key[new_key.len().saturating_sub(4)..]
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

        anyhow::bail!(
            "All providers/models failed. Attempts:\n{}",
            failures.join("\n")
        )
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
        // Try each provider/model combination for streaming
        // For streaming, we use the first provider that supports it and has streaming enabled
        for (provider_name, provider) in &self.providers {
            if !provider.supports_streaming() || !options.enabled {
                continue;
            }

            // Clone provider data for the stream
            let provider_clone = provider_name.clone();

            // Try the first model in the chain for streaming
            let current_model = match self.model_chain(model).first() {
                Some(m) => m.to_string(),
                None => model.to_string(),
            };

            // For streaming, we attempt once and propagate errors
            // The caller can retry the entire request if needed
            let stream = provider.stream_chat_with_system(
                system_prompt,
                message,
                &current_model,
                temperature,
                options,
            );

            // Use a channel to bridge the stream with logging
            let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

            tokio::spawn(async move {
                let mut stream = stream;
                while let Some(chunk) = stream.next().await {
                    if let Err(ref e) = chunk {
                        tracing::warn!(
                            provider = provider_clone,
                            model = current_model,
                            "Streaming error: {e}"
                        );
                    }
                    if tx.send(chunk).await.is_err() {
                        break; // Receiver dropped
                    }
                }
            });

            // Convert channel receiver to stream
            return stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|chunk| (chunk, rx))
            })
            .boxed();
        }

        // No streaming support available
        stream::once(async move {
            Err(super::traits::StreamError::Provider(
                "No provider supports streaming".to_string(),
            ))
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct MockProvider {
        calls: Arc<AtomicUsize>,
        fail_until_attempt: usize,
        response: &'static str,
        error: &'static str,
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt <= self.fail_until_attempt {
                anyhow::bail!(self.error);
            }
            Ok(self.response.to_string())
        }

        async fn chat_with_history(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt <= self.fail_until_attempt {
                anyhow::bail!(self.error);
            }
            Ok(self.response.to_string())
        }
    }

    /// Mock that records which model was used for each call.
    struct ModelAwareMock {
        calls: Arc<AtomicUsize>,
        models_seen: parking_lot::Mutex<Vec<String>>,
        fail_models: Vec<&'static str>,
        response: &'static str,
    }

    #[async_trait]
    impl Provider for ModelAwareMock {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.models_seen.lock().push(model.to_string());
            if self.fail_models.contains(&model) {
                anyhow::bail!("500 model {} unavailable", model);
            }
            Ok(self.response.to_string())
        }
    }

    // ── Existing tests (preserved) ──

    #[tokio::test]
    async fn succeeds_without_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: 0,
                    response: "ok",
                    error: "boom",
                }),
            )],
            2,
            1,
        );

        let result = provider.simple_chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_then_recovers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: 1,
                    response: "recovered",
                    error: "temporary",
                }),
            )],
            2,
            1,
        );

        let result = provider.simple_chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "recovered");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn falls_back_after_retries_exhausted() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));

        let provider = ReliableProvider::new(
            vec![
                (
                    "primary".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&primary_calls),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "primary down",
                    }),
                ),
                (
                    "fallback".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&fallback_calls),
                        fail_until_attempt: 0,
                        response: "from fallback",
                        error: "fallback down",
                    }),
                ),
            ],
            1,
            1,
        );

        let result = provider.simple_chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "from fallback");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn returns_aggregated_error_when_all_providers_fail() {
        let provider = ReliableProvider::new(
            vec![
                (
                    "p1".into(),
                    Box::new(MockProvider {
                        calls: Arc::new(AtomicUsize::new(0)),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "p1 error",
                    }),
                ),
                (
                    "p2".into(),
                    Box::new(MockProvider {
                        calls: Arc::new(AtomicUsize::new(0)),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "p2 error",
                    }),
                ),
            ],
            0,
            1,
        );

        let err = provider
            .simple_chat("hello", "test", 0.0)
            .await
            .expect_err("all providers should fail");
        let msg = err.to_string();
        assert!(msg.contains("All providers/models failed"));
        assert!(msg.contains("provider=p1 model=test"));
        assert!(msg.contains("provider=p2 model=test"));
        assert!(msg.contains("error=p1 error"));
        assert!(msg.contains("error=p2 error"));
        assert!(msg.contains("retryable"));
    }

    #[test]
    fn non_retryable_detects_common_patterns() {
        assert!(is_non_retryable(&anyhow::anyhow!("400 Bad Request")));
        assert!(is_non_retryable(&anyhow::anyhow!("401 Unauthorized")));
        assert!(is_non_retryable(&anyhow::anyhow!("403 Forbidden")));
        assert!(is_non_retryable(&anyhow::anyhow!("404 Not Found")));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "invalid api key provided"
        )));
        assert!(is_non_retryable(&anyhow::anyhow!("authentication failed")));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "model glm-4.7 not found"
        )));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "unsupported model: glm-4.7"
        )));
        assert!(!is_non_retryable(&anyhow::anyhow!("429 Too Many Requests")));
        assert!(!is_non_retryable(&anyhow::anyhow!("408 Request Timeout")));
        assert!(!is_non_retryable(&anyhow::anyhow!(
            "500 Internal Server Error"
        )));
        assert!(!is_non_retryable(&anyhow::anyhow!("502 Bad Gateway")));
        assert!(!is_non_retryable(&anyhow::anyhow!("timeout")));
        assert!(!is_non_retryable(&anyhow::anyhow!("connection reset")));
        assert!(!is_non_retryable(&anyhow::anyhow!(
            "model overloaded, try again later"
        )));
        assert!(is_non_retryable(&anyhow::anyhow!(
            "OpenAI Codex stream error: Your input exceeds the context window of this model."
        )));
    }

    #[tokio::test]
    async fn context_window_error_aborts_retries_and_model_fallbacks() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut model_fallbacks = std::collections::HashMap::new();
        model_fallbacks.insert(
            "gpt-5.3-codex".to_string(),
            vec!["gpt-5.2-codex".to_string()],
        );

        let provider = ReliableProvider::new(
            vec![(
                "openai-codex".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: usize::MAX,
                    response: "never",
                    error: "OpenAI Codex stream error: Your input exceeds the context window of this model. Please adjust your input and try again.",
                }),
            )],
            4,
            1,
        )
        .with_model_fallbacks(model_fallbacks);

        let err = provider
            .simple_chat("hello", "gpt-5.3-codex", 0.0)
            .await
            .expect_err("context window overflow should fail fast");
        let msg = err.to_string();

        assert!(msg.contains("context window"));
        assert!(msg.contains("skipped"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn aggregated_error_marks_non_retryable_model_mismatch_with_details() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "custom".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: usize::MAX,
                    response: "never",
                    error: "unsupported model: glm-4.7",
                }),
            )],
            3,
            1,
        );

        let err = provider
            .simple_chat("hello", "glm-4.7", 0.0)
            .await
            .expect_err("provider should fail");
        let msg = err.to_string();

        assert!(msg.contains("non_retryable"));
        assert!(msg.contains("error=unsupported model: glm-4.7"));
        // Non-retryable errors should not consume retry budget.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn skips_retries_on_non_retryable_error() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));

        let provider = ReliableProvider::new(
            vec![
                (
                    "primary".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&primary_calls),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "401 Unauthorized",
                    }),
                ),
                (
                    "fallback".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&fallback_calls),
                        fail_until_attempt: 0,
                        response: "from fallback",
                        error: "fallback err",
                    }),
                ),
            ],
            3,
            1,
        );

        let result = provider.simple_chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "from fallback");
        // Primary should have been called only once (no retries)
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn chat_with_history_retries_then_recovers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: 1,
                    response: "history ok",
                    error: "temporary",
                }),
            )],
            2,
            1,
        );

        let messages = vec![ChatMessage::system("system"), ChatMessage::user("hello")];
        let result = provider
            .chat_with_history(&messages, "test", 0.0)
            .await
            .unwrap();
        assert_eq!(result, "history ok");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn chat_with_history_falls_back() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));

        let provider = ReliableProvider::new(
            vec![
                (
                    "primary".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&primary_calls),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "primary down",
                    }),
                ),
                (
                    "fallback".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&fallback_calls),
                        fail_until_attempt: 0,
                        response: "fallback ok",
                        error: "fallback err",
                    }),
                ),
            ],
            1,
            1,
        );

        let messages = vec![ChatMessage::user("hello")];
        let result = provider
            .chat_with_history(&messages, "test", 0.0)
            .await
            .unwrap();
        assert_eq!(result, "fallback ok");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }

    // ── New tests: model failover ──

    #[tokio::test]
    async fn model_failover_tries_fallback_model() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mock = Arc::new(ModelAwareMock {
            calls: Arc::clone(&calls),
            models_seen: parking_lot::Mutex::new(Vec::new()),
            fail_models: vec!["claude-opus"],
            response: "ok from sonnet",
        });

        let mut fallbacks = HashMap::new();
        fallbacks.insert("claude-opus".to_string(), vec!["claude-sonnet".to_string()]);

        let provider = ReliableProvider::new(
            vec![(
                "anthropic".into(),
                Box::new(mock.clone()) as Box<dyn Provider>,
            )],
            0, // no retries — force immediate model failover
            1,
        )
        .with_model_fallbacks(fallbacks);

        let result = provider
            .simple_chat("hello", "claude-opus", 0.0)
            .await
            .unwrap();
        assert_eq!(result, "ok from sonnet");

        let seen = mock.models_seen.lock();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], "claude-opus");
        assert_eq!(seen[1], "claude-sonnet");
    }

    #[tokio::test]
    async fn model_failover_all_models_fail() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mock = Arc::new(ModelAwareMock {
            calls: Arc::clone(&calls),
            models_seen: parking_lot::Mutex::new(Vec::new()),
            fail_models: vec!["model-a", "model-b", "model-c"],
            response: "never",
        });

        let mut fallbacks = HashMap::new();
        fallbacks.insert(
            "model-a".to_string(),
            vec!["model-b".to_string(), "model-c".to_string()],
        );

        let provider = ReliableProvider::new(
            vec![("p1".into(), Box::new(mock.clone()) as Box<dyn Provider>)],
            0,
            1,
        )
        .with_model_fallbacks(fallbacks);

        let err = provider
            .simple_chat("hello", "model-a", 0.0)
            .await
            .expect_err("all models should fail");
        assert!(err.to_string().contains("All providers/models failed"));

        let seen = mock.models_seen.lock();
        assert_eq!(seen.len(), 3);
    }

    #[tokio::test]
    async fn no_model_fallbacks_behaves_like_before() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: 0,
                    response: "ok",
                    error: "boom",
                }),
            )],
            2,
            1,
        );
        // No model_fallbacks set — should work exactly as before
        let result = provider.simple_chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // ── New tests: auth rotation ──

    #[tokio::test]
    async fn auth_rotation_cycles_keys() {
        let provider = ReliableProvider::new(
            vec![(
                "p".into(),
                Box::new(MockProvider {
                    calls: Arc::new(AtomicUsize::new(0)),
                    fail_until_attempt: 0,
                    response: "ok",
                    error: "",
                }),
            )],
            0,
            1,
        )
        .with_api_keys(vec!["key-a".into(), "key-b".into(), "key-c".into()]);

        // Rotate 5 times, verify round-robin
        let keys: Vec<&str> = (0..5).map(|_| provider.rotate_key().unwrap()).collect();
        assert_eq!(keys, vec!["key-a", "key-b", "key-c", "key-a", "key-b"]);
    }

    #[tokio::test]
    async fn auth_rotation_returns_none_when_empty() {
        let provider = ReliableProvider::new(vec![], 0, 1);
        assert!(provider.rotate_key().is_none());
    }

    // ── New tests: Retry-After parsing ──

    #[test]
    fn parse_retry_after_integer() {
        let err = anyhow::anyhow!("429 Too Many Requests, Retry-After: 5");
        assert_eq!(parse_retry_after_ms(&err), Some(5000));
    }

    #[test]
    fn parse_retry_after_float() {
        let err = anyhow::anyhow!("Rate limited. retry_after: 2.5 seconds");
        assert_eq!(parse_retry_after_ms(&err), Some(2500));
    }

    #[test]
    fn parse_retry_after_missing() {
        let err = anyhow::anyhow!("500 Internal Server Error");
        assert_eq!(parse_retry_after_ms(&err), None);
    }

    #[test]
    fn rate_limited_detection() {
        assert!(is_rate_limited(&anyhow::anyhow!("429 Too Many Requests")));
        assert!(is_rate_limited(&anyhow::anyhow!(
            "HTTP 429 rate limit exceeded"
        )));
        assert!(!is_rate_limited(&anyhow::anyhow!("401 Unauthorized")));
        assert!(!is_rate_limited(&anyhow::anyhow!(
            "500 Internal Server Error"
        )));
    }

    #[test]
    fn non_retryable_rate_limit_detects_plan_restricted_model() {
        let err = anyhow::anyhow!(
            "{}",
            "API error (429 Too Many Requests): {\"code\":1311,\"message\":\"the current account plan does not include glm-5\"}"
        );
        assert!(
            is_non_retryable_rate_limit(&err),
            "plan-restricted 429 should skip retries"
        );
    }

    #[test]
    fn non_retryable_rate_limit_detects_insufficient_balance() {
        let err = anyhow::anyhow!(
            "{}",
            "API error (429 Too Many Requests): {\"code\":1113,\"message\":\"insufficient balance\"}"
        );
        assert!(
            is_non_retryable_rate_limit(&err),
            "insufficient-balance 429 should skip retries"
        );
    }

    #[test]
    fn non_retryable_rate_limit_does_not_flag_generic_429() {
        let err = anyhow::anyhow!("429 Too Many Requests: rate limit exceeded");
        assert!(
            !is_non_retryable_rate_limit(&err),
            "generic rate-limit 429 should remain retryable"
        );
    }

    #[test]
    fn compute_backoff_uses_retry_after() {
        let provider = ReliableProvider::new(vec![], 0, 500);
        let err = anyhow::anyhow!("429 Retry-After: 3");
        assert_eq!(provider.compute_backoff(500, &err), 3000);
    }

    #[test]
    fn compute_backoff_caps_at_30s() {
        let provider = ReliableProvider::new(vec![], 0, 500);
        let err = anyhow::anyhow!("429 Retry-After: 120");
        assert_eq!(provider.compute_backoff(500, &err), 30_000);
    }

    #[test]
    fn compute_backoff_falls_back_to_base() {
        let provider = ReliableProvider::new(vec![], 0, 500);
        let err = anyhow::anyhow!("500 Server Error");
        assert_eq!(provider.compute_backoff(500, &err), 500);
    }

    // ── §2.1 API auth error (401/403) tests ──────────────────

    #[test]
    fn non_retryable_detects_401() {
        let err = anyhow::anyhow!("API error (401 Unauthorized): invalid api key");
        assert!(
            is_non_retryable(&err),
            "401 errors must be detected as non-retryable"
        );
    }

    #[test]
    fn non_retryable_detects_403() {
        let err = anyhow::anyhow!("API error (403 Forbidden): access denied");
        assert!(
            is_non_retryable(&err),
            "403 errors must be detected as non-retryable"
        );
    }

    #[test]
    fn non_retryable_detects_404() {
        let err = anyhow::anyhow!("API error (404 Not Found): model not found");
        assert!(
            is_non_retryable(&err),
            "404 errors must be detected as non-retryable"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_429() {
        let err = anyhow::anyhow!("429 Too Many Requests");
        assert!(
            !is_non_retryable(&err),
            "429 must NOT be treated as non-retryable (it is retryable with backoff)"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_408() {
        let err = anyhow::anyhow!("408 Request Timeout");
        assert!(
            !is_non_retryable(&err),
            "408 must NOT be treated as non-retryable (it is retryable)"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_500() {
        let err = anyhow::anyhow!("500 Internal Server Error");
        assert!(
            !is_non_retryable(&err),
            "500 must NOT be treated as non-retryable (server errors are retryable)"
        );
    }

    #[test]
    fn non_retryable_does_not_flag_502() {
        let err = anyhow::anyhow!("502 Bad Gateway");
        assert!(
            !is_non_retryable(&err),
            "502 must NOT be treated as non-retryable"
        );
    }

    // ── §2.2 Rate limit Retry-After edge cases ───────────────

    #[test]
    fn parse_retry_after_zero() {
        let err = anyhow::anyhow!("429 Too Many Requests, Retry-After: 0");
        assert_eq!(
            parse_retry_after_ms(&err),
            Some(0),
            "Retry-After: 0 should parse as 0ms"
        );
    }

    #[test]
    fn parse_retry_after_with_underscore_separator() {
        let err = anyhow::anyhow!("rate limited, retry_after: 10");
        assert_eq!(
            parse_retry_after_ms(&err),
            Some(10_000),
            "retry_after with underscore must be parsed"
        );
    }

    #[test]
    fn parse_retry_after_space_separator() {
        let err = anyhow::anyhow!("Retry-After 7");
        assert_eq!(
            parse_retry_after_ms(&err),
            Some(7000),
            "Retry-After with space separator must be parsed"
        );
    }

    #[test]
    fn rate_limited_false_for_generic_error() {
        let err = anyhow::anyhow!("Connection refused");
        assert!(
            !is_rate_limited(&err),
            "generic errors must not be flagged as rate-limited"
        );
    }

    // ── §2.3 Malformed API response error classification ─────

    #[tokio::test]
    async fn non_retryable_skips_retries_for_401() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: usize::MAX,
                    response: "never",
                    error: "API error (401 Unauthorized): invalid key",
                }),
            )],
            5,
            1,
        );

        let result = provider.simple_chat("hello", "test", 0.0).await;
        assert!(result.is_err(), "401 should fail without retries");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "must not retry on 401 — should be exactly 1 call"
        );
    }

    #[tokio::test]
    async fn non_retryable_rate_limit_skips_retries_for_plan_errors() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: usize::MAX,
                    response: "never",
                    error: "API error (429 Too Many Requests): {\"code\":1311,\"message\":\"plan does not include glm-5\"}",
                }),
            )],
            5,
            1,
        );

        let result = provider.simple_chat("hello", "test", 0.0).await;
        assert!(
            result.is_err(),
            "plan-restricted 429 should fail quickly without retrying"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "must not retry non-retryable 429 business errors"
        );
    }

    // ── Arc<ModelAwareMock> Provider impl for test ──

    #[async_trait]
    impl Provider for Arc<ModelAwareMock> {
        async fn chat_with_system(
            &self,
            system_prompt: Option<&str>,
            message: &str,
            model: &str,
            temperature: f64,
        ) -> anyhow::Result<String> {
            self.as_ref()
                .chat_with_system(system_prompt, message, model, temperature)
                .await
        }
    }
}
