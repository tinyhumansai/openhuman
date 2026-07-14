use super::*;
use crate::openhuman::inference::provider::traits::StreamError;
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
async fn records_successful_fallback_provider_route() {
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
                    response: "ok",
                    error: "boom",
                }),
            ),
        ],
        0,
        1,
    );

    let recorded =
        crate::openhuman::inference::provider::with_resolved_provider_route_scope(async {
            let result = provider
                .chat_with_system(Some("system"), "hello", "requested-model", 0.0)
                .await
                .unwrap();
            assert_eq!(result, "ok");
            crate::openhuman::inference::provider::current_resolved_provider_route()
        })
        .await
        .expect("reliable provider should record the successful route");

    assert_eq!(recorded.provider, "fallback");
    assert_eq!(recorded.model, "requested-model");
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
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
async fn session_expired_aborts_retries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ReliableProvider::new(
        vec![(
            "openhuman".into(),
            Box::new(MockProvider {
                calls: Arc::clone(&calls),
                fail_until_attempt: usize::MAX,
                response: "never",
                error: "SESSION_EXPIRED: backend session not active — sign in to resume LLM work",
            }),
        )],
        3,
        1,
    );

    let err = provider
        .simple_chat("hello", "reasoning-v1", 0.0)
        .await
        .expect_err("session-expired should fail fast");
    let msg = err.to_string();

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "session-expired must skip retry loop"
    );
    assert!(
        msg.contains("non_retryable"),
        "aggregate should classify SESSION_EXPIRED as non_retryable: {msg}"
    );
    assert!(
        !msg.contains("attempt 2/4"),
        "aggregate should contain only the first attempt for this provider: {msg}"
    );
}

/// Streaming-path mock that emits a single configurable `StreamError::Provider`
/// then ends, and tracks how many times the stream was created (`stream_calls`)
/// and how many times the consumer polled it (`polls`). The latter is the
/// signal used by [`session_expired_aborts_retries_streaming`] to prove that
/// `is_stream_error_non_retryable` broke the retry loop after the first error
/// instead of polling for further attempts.
struct StreamingErrorMock {
    stream_calls: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    error: &'static str,
}

#[async_trait]
impl Provider for StreamingErrorMock {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!(self.error)
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!(self.error)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn stream_chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
        _options: StreamOptions,
    ) -> futures_util::stream::BoxStream<'static, StreamResult<StreamChunk>> {
        use futures_util::{stream, StreamExt};
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        let polls = Arc::clone(&self.polls);
        let error = self.error.to_string();
        // `unfold` state: `sent` flips to true after the first poll. The
        // counter bumps on every poll so the test can prove that the retry
        // loop short-circuited after the first error (polls == 1) rather
        // than continuing to drain (polls == 2).
        stream::unfold(false, move |sent| {
            let polls = Arc::clone(&polls);
            let error = error.clone();
            async move {
                polls.fetch_add(1, Ordering::SeqCst);
                if sent {
                    None
                } else {
                    Some((Err(StreamError::Provider(error)), true))
                }
            }
        })
        .boxed()
    }
}

#[tokio::test]
async fn session_expired_aborts_retries_streaming() {
    use futures_util::StreamExt;

    let stream_calls = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let provider = ReliableProvider::new(
        vec![(
            "openhuman".into(),
            Box::new(StreamingErrorMock {
                stream_calls: Arc::clone(&stream_calls),
                polls: Arc::clone(&polls),
                error: "SESSION_EXPIRED: backend session not active — sign in to resume LLM work",
            }),
        )],
        3,
        1,
    );

    let mut stream = provider.stream_chat_with_system(
        None,
        "hello",
        "reasoning-v1",
        0.0,
        StreamOptions::new(true),
    );

    // Drain the consumer-facing stream. ReliableProvider does NOT forward
    // candidate errors — the consumer only sees a single terminal
    // "All streaming providers/models failed" once retries are exhausted.
    let mut terminal: Option<String> = None;
    while let Some(item) = stream.next().await {
        if let Err(StreamError::Provider(msg)) = item {
            terminal = Some(msg);
        }
    }

    assert_eq!(
        stream_calls.load(Ordering::SeqCst),
        1,
        "single candidate (one provider, one model) must build exactly one stream"
    );
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "session-expired must abort the streaming retry loop after the first poll; \
         a second poll means is_stream_error_non_retryable misclassified it"
    );
    let terminal = terminal.expect("stream must surface a terminal aggregate error");
    assert!(
        terminal.contains("All streaming providers/models failed"),
        "expected aggregate failure terminal, got: {terminal}"
    );
}

/// Streaming mock whose stream fails with a *retryable* `StreamError` for the
/// first `fail_until` creations and then yields a single successful chunk. Each
/// stream is mpsc-like (exhausts after one item), exactly like the real
/// provider impl — so a retry that re-polls the same dead stream would see
/// `None` and give up. `stream_calls` records how many times a stream was
/// created, the signal used to prove lazy creation (audit C2) and
/// recreate-on-retry (audit C6).
struct StreamingRetryMock {
    stream_calls: Arc<AtomicUsize>,
    fail_until: usize,
}

#[async_trait]
impl Provider for StreamingRetryMock {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!("unused")
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!("unused")
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn stream_chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
        _options: StreamOptions,
    ) -> futures_util::stream::BoxStream<'static, StreamResult<StreamChunk>> {
        use futures_util::{stream, StreamExt};
        // The Nth stream creation (1-based) fails if N <= fail_until, else
        // succeeds. Firing the HTTP request happens *here*, so counting
        // creations is the proxy for "requests issued".
        let n = self.stream_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let succeed = n > self.fail_until;
        stream::once(async move {
            if succeed {
                Ok(StreamChunk::delta("hello"))
            } else {
                // A generic provider error — retryable per
                // is_stream_error_non_retryable.
                Err(StreamError::Provider("transient upstream blip".to_string()))
            }
        })
        .boxed()
    }
}

/// C2: streaming failover must NOT pre-fire every provider×model stream. With a
/// 2-model fallback chain and a provider that succeeds on the first attempt,
/// only ONE stream may be created (the winning candidate) — not the full
/// cartesian product.
#[tokio::test]
async fn streaming_does_not_prefire_all_candidates() {
    use futures_util::StreamExt;

    let stream_calls = Arc::new(AtomicUsize::new(0));
    let mut fallbacks = HashMap::new();
    fallbacks.insert(
        "model-a".to_string(),
        vec!["model-b".to_string(), "model-c".to_string()],
    );

    let provider = ReliableProvider::new(
        vec![(
            "p".into(),
            Box::new(StreamingRetryMock {
                stream_calls: Arc::clone(&stream_calls),
                fail_until: 0, // succeed immediately
            }),
        )],
        3,
        1,
    )
    .with_model_fallbacks(fallbacks);

    let mut stream =
        provider.stream_chat_with_system(None, "hi", "model-a", 0.0, StreamOptions::new(true));

    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok(chunk) = item {
            chunks.push(chunk.delta);
        }
    }

    assert_eq!(chunks, vec!["hello".to_string()]);
    assert_eq!(
        stream_calls.load(Ordering::SeqCst),
        1,
        "only the winning candidate may create a stream; the rest must stay lazy (C2)"
    );
}

/// C6: a retryable streaming failure must RE-CREATE the candidate stream on the
/// next attempt rather than re-poll the already-exhausted one. With
/// `fail_until = 2` and `max_retries = 3`, the same provider/model is attempted
/// up to 4 times; creations 1 and 2 fail, creation 3 succeeds. If the retry
/// loop re-polled the dead stream instead of recreating it, we'd only ever see
/// a single creation and the call would fail.
#[tokio::test]
async fn streaming_retry_recreates_stream() {
    use futures_util::StreamExt;

    let stream_calls = Arc::new(AtomicUsize::new(0));
    let provider = ReliableProvider::new(
        vec![(
            "p".into(),
            Box::new(StreamingRetryMock {
                stream_calls: Arc::clone(&stream_calls),
                fail_until: 2,
            }),
        )],
        3,
        1,
    );

    let mut stream =
        provider.stream_chat_with_system(None, "hi", "reasoning-v1", 0.0, StreamOptions::new(true));

    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok(chunk) = item {
            chunks.push(chunk.delta);
        }
    }

    assert_eq!(
        chunks,
        vec!["hello".to_string()],
        "retry must eventually recover once the recreated stream succeeds (C6)"
    );
    assert_eq!(
        stream_calls.load(Ordering::SeqCst),
        3,
        "each retry attempt must recreate the candidate stream (C6), not re-poll the dead one"
    );
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

// ── issue #1596: custom_openai model-not-found UX ──
//
// When a `custom_openai` provider is configured with a model name that
// does not exist on the user's endpoint (e.g. `reasoning-v1` on a
// provider that never shipped it), the bail aggregate is the only
// signal the user has — and the default text was an opaque dump of
// per-attempt error envelopes. The helper below tags the dump with a
// pointer at `reliability.model_fallbacks` when the user hasn't
// configured a chain yet, so the next step is obvious without
// re-reading the docs.

/// End-to-end: a `chat_with_system` call that fails with the
/// `custom_openai`-shaped 404 must bail with the user-actionable hint
/// included.
#[tokio::test]
async fn chat_with_system_bail_includes_hint_when_no_fallbacks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ReliableProvider::new(
        vec![(
            "custom_openai".into(),
            Box::new(MockProvider {
                calls: Arc::clone(&calls),
                fail_until_attempt: 999, // never recovers
                response: "(unused)",
                error: "custom_openai API error (404 Not Found): \
                        {\"error\":{\"message\":\"model 'reasoning-v1' not found\",\
                        \"type\":\"not_found_error\"}}",
            }),
        )],
        0,
        1,
    );

    let err = provider
        .chat_with_system(None, "hi", "reasoning-v1", 0.0)
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("may not be available on your provider"),
        "expected hint, got: {err}"
    );
    assert!(
        err.contains("reasoning-v1"),
        "expected model name in error: {err}"
    );
}

/// End-to-end: when the user has configured a fallback chain and it
/// also exhausts, the hint must NOT fire — the user already knows the
/// knob exists, they just need the raw dump to debug their chain.
#[tokio::test]
async fn chat_with_system_bail_omits_hint_when_fallbacks_configured_but_all_fail() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut fallbacks = HashMap::new();
    fallbacks.insert(
        "reasoning-v1".to_string(),
        vec!["chat-v1".to_string(), "general-v1".to_string()],
    );

    let provider = ReliableProvider::new(
        vec![(
            "custom_openai".into(),
            Box::new(MockProvider {
                calls: Arc::clone(&calls),
                fail_until_attempt: 999,
                response: "(unused)",
                error: "custom_openai API error (404 Not Found): model not found",
            }),
        )],
        0,
        1,
    )
    .with_model_fallbacks(fallbacks);

    let err = provider
        .chat_with_system(None, "hi", "reasoning-v1", 0.0)
        .await
        .unwrap_err()
        .to_string();

    assert!(
        !err.contains("Configure a fallback chain"),
        "must not nag when chain already configured: {err}"
    );
    // All three models in chain (configured + 2 fallbacks) must have
    // been attempted; the dump is the user's diagnostic surface.
    assert!(
        err.contains("reasoning-v1") && err.contains("chat-v1") && err.contains("general-v1"),
        "expected dump to mention every model tried: {err}"
    );
}
