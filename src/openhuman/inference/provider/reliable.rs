use super::traits::{
    ChatMessage, ChatRequest, ChatResponse, StreamChunk, StreamError, StreamOptions, StreamResult,
};
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
    let msg = err.to_string();
    // Session-expired is a user-auth-state boundary condition, not a
    // transient provider outage. Retrying just burns attempts and delays
    // the sign-in prompt.
    if crate::core::observability::is_session_expired_message(&msg) {
        return true;
    }

    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            let code = status.as_u16();
            return status.is_client_error() && code != 429 && code != 408;
        }
    }
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

/// Classify a StreamError without losing type information.
/// Inspects the inner reqwest::Error status directly for Http variants.
fn is_stream_error_non_retryable(err: &StreamError) -> bool {
    match err {
        StreamError::Http(reqwest_err) => {
            if let Some(status) = reqwest_err.status() {
                let code = status.as_u16();
                // Client errors except 429 (rate limit) and 408 (timeout) are non-retryable
                return status.is_client_error() && code != 429 && code != 408;
            }
            false
        }
        StreamError::Provider(msg) => {
            // Mirror the non-streaming classifier: session-expired is a
            // user-auth-state boundary, not a transient provider outage —
            // fail fast so the streaming caller can prompt sign-in instead
            // of burning the retry budget.
            if crate::core::observability::is_session_expired_message(msg) {
                return true;
            }
            let lower = msg.to_lowercase();
            lower.contains("invalid api key")
                || lower.contains("unauthorized")
                || lower.contains("forbidden")
                || lower.contains("model")
                    && (lower.contains("not found") || lower.contains("unsupported"))
        }
        // JSON/SSE parse errors and IO errors are generally non-retryable
        StreamError::Json(_) | StreamError::InvalidSse(_) => true,
        StreamError::Io(_) => false,
    }
}

fn is_context_window_exceeded(err: &anyhow::Error) -> bool {
    // Single source of truth for the context-overflow phrasing lives in
    // `ops::is_context_window_exceeded_message` so the non-retryable
    // classifier here, the `api_error` Sentry-suppression cascade, and the
    // `core::observability` `ContextWindowExceeded` arm can't drift apart.
    super::is_context_window_exceeded_message(&err.to_string())
}

/// Detect provider-side temporary capacity/outage errors. Covers:
///
/// - HTTP `408 Request Timeout`, `502 Bad Gateway`, `503 Service Unavailable`,
///   `504 Gateway Timeout` — both via direct `reqwest::Error` downcast and via
///   the formatted `"<provider> API error (<status>): …"` text emitted by
///   `ops::api_error` (the path that actually reaches `report_error`).
/// - Provider-agnostic text markers like `"no healthy upstream"` /
///   `"upstream unavailable"` that don't come with a typed status.
///
/// Pairs with [`is_rate_limited`] which handles 429 separately. Together they
/// form the transient-classifier the tool-call loop uses before deciding
/// whether to push a per-attempt event to Sentry (see OPENHUMAN-TAURI-2E /
/// -84 / -T / -G classes — per-iteration noise from upstream throttling).
///
/// **Status list maintenance note**: the codes matched below (408/502/503/504)
/// are a subset of
/// [`crate::core::observability::TRANSIENT_PROVIDER_HTTP_STATUSES`] — that
/// const is the single source of truth for the `before_send` filter and the
/// call-site classifier in `providers/ops.rs`. We don't reference the const
/// directly here because this function takes a different code path (anyhow
/// error downcast vs typed `reqwest::StatusCode`) and because 429 is split out
/// into `is_rate_limited` (with its own retry-after parsing). If a new
/// transient status is added to the const, **also add it to this `matches!`
/// arm and the text-pattern list below**.
///
/// Note: 429 lives in `TRANSIENT_PROVIDER_HTTP_STATUSES` but is intentionally
/// absent here — `is_rate_limited` handles it separately because 429 responses
/// may carry a `Retry-After` header that `parse_retry_after_ms` uses to pick a
/// precise backoff rather than the default exponential schedule.
pub(crate) fn is_upstream_unhealthy(err: &anyhow::Error) -> bool {
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            if matches!(status.as_u16(), 408 | 502 | 503 | 504) {
                return true;
            }
        }
    }
    let lower = err.to_string().to_lowercase();
    lower.contains("no healthy upstream")
        || lower.contains("upstream unavailable")
        || lower.contains("service unavailable")
        || lower.contains("503 service unavailable")
        || lower.contains("408 request timeout")
        || lower.contains("502 bad gateway")
        || lower.contains("504 gateway timeout")
}

/// Check if an error is a rate-limit (429) error.
pub(crate) fn is_rate_limited(err: &anyhow::Error) -> bool {
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
pub(crate) fn parse_retry_after_ms(err: &anyhow::Error) -> Option<u64> {
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

fn failure_reason(
    rate_limited: bool,
    non_retryable: bool,
    upstream_unhealthy: bool,
) -> &'static str {
    if upstream_unhealthy {
        "upstream_unhealthy"
    } else if rate_limited && non_retryable {
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
    super::sanitize_api_error(&super::format_anyhow_chain(err))
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

fn rotated_key_log_detail(after_rotate_index: usize, total: usize) -> String {
    let slot = if total == 0 {
        0
    } else {
        after_rotate_index.saturating_sub(1) % total + 1
    };
    format!("slot={slot}/{total}")
}

/// Format the final bail message produced when every provider+model in the
/// chain has failed.
///
/// When the originally-requested `model` has no fallback chain configured
/// in `model_fallbacks`, prepend a single user-actionable hint pointing at
/// the most common cause we see in production (OPENHUMAN-TAURI-BY / -BZ /
/// -C0 / -C1, issue #1596): the user has wired up a `custom_openai`
/// provider whose endpoint does not expose the configured `default_model`.
/// In that scenario the bail aggregate is otherwise an opaque stack of
/// provider-formatted error envelopes which gives the user no clue where
/// to look.
///
/// We deliberately avoid emitting the hint when fallbacks *are* configured
/// — the user has already engaged with the knob and likely has either a
/// real outage or a misconfigured chain; the dump-of-attempts surface is
/// what they need to debug it.
fn format_failure_aggregate(
    model: &str,
    failures: &[String],
    has_configured_fallbacks: bool,
) -> String {
    let attempts = format!(
        "All providers/models failed. Attempts:\n{}",
        failures.join("\n")
    );
    if has_configured_fallbacks {
        attempts
    } else {
        format!(
            "The model `{model}` may not be available on your provider. \
             Configure a fallback chain via `reliability.model_fallbacks` in your \
             OpenHuman config, or change your default model in Settings → AI.\n\n{attempts}"
        )
    }
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
                    };
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

        // Collect provider streams lazily inside the task — we need owned data
        // Provider trait is object-safe, so we call stream_chat_with_system per attempt
        // We need to pre-create all possible streams since Provider is behind &self
        // Instead, collect the streams for each provider+model combo upfront
        let mut candidate_streams: Vec<(
            String,
            String,
            stream::BoxStream<'static, StreamResult<StreamChunk>>,
        )> = Vec::new();
        for current_model in &model_chain {
            for (provider_name, provider) in &streaming_providers {
                let s = provider.stream_chat_with_system(
                    system_prompt,
                    message,
                    current_model,
                    temperature,
                    options,
                );
                candidate_streams.push(((*provider_name).clone(), current_model.clone(), s));
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);
        let max_retries = self.max_retries;

        tokio::spawn(async move {
            for (provider_name, current_model, mut candidate_stream) in candidate_streams {
                let mut backoff_ms = base_backoff_ms;
                let mut attempts = 0u32;

                loop {
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
                            // Continue inner loop — stream may yield more items
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
