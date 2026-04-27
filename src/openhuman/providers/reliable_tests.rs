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

// ── upstream_unhealthy classification and failure_reason precedence ──

#[test]
fn upstream_unhealthy_detects_no_healthy_upstream() {
    let err = anyhow::anyhow!("no healthy upstream available");
    assert!(is_upstream_unhealthy(&err));
}

#[test]
fn upstream_unhealthy_detects_upstream_unavailable() {
    let err = anyhow::anyhow!("upstream unavailable: backend down");
    assert!(is_upstream_unhealthy(&err));
}

#[test]
fn upstream_unhealthy_detects_service_unavailable() {
    let err = anyhow::anyhow!("503 service unavailable");
    assert!(is_upstream_unhealthy(&err));
}

#[test]
fn upstream_unhealthy_does_not_flag_generic_error() {
    let err = anyhow::anyhow!("timeout after 30s");
    assert!(!is_upstream_unhealthy(&err));
}

#[test]
fn failure_reason_upstream_unhealthy_wins_over_rate_limited() {
    // Both rate_limited AND upstream_unhealthy — upstream_unhealthy must win.
    assert_eq!(failure_reason(true, false, true), "upstream_unhealthy");
}

#[test]
fn failure_reason_upstream_unhealthy_wins_over_non_retryable() {
    // Both non_retryable AND upstream_unhealthy — upstream_unhealthy must win.
    assert_eq!(failure_reason(false, true, true), "upstream_unhealthy");
}

#[test]
fn failure_reason_upstream_unhealthy_wins_over_all_others() {
    // All flags set — upstream_unhealthy must still win.
    assert_eq!(failure_reason(true, true, true), "upstream_unhealthy");
}
