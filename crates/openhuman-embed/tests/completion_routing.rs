//! Ordered routing, capped truncation retries and provider-reported accounting.
use openhuman_embed::routing::{CompletionLadder, CompletionRung, TruncationRetry};
use openhuman_embed::{ChatMessage, Completer, CompletionRequest, Route};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

struct Script(Vec<Value>, AtomicUsize);
impl Respond for Script {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let index = self.1.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(self.0[index.min(self.0.len() - 1)].clone())
    }
}
fn answer(model: &str, finish: &str, cost: f64) -> Value {
    json!({"model":model,"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":finish}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"cost":cost}})
}
async fn scripted(bodies: Vec<Value>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(Script(bodies, AtomicUsize::new(0)))
        .mount(&server)
        .await;
    server
}
fn rung(server: &MockServer, model: &str) -> CompletionRung {
    CompletionRung::new(
        Completer::new(Route::openai_compatible(
            format!("{}/v1", server.uri()),
            "fixture",
        )),
        model,
    )
}
#[tokio::test]
async fn truncation_doubles_the_cap_before_ordered_unpinned_fallback() {
    let first = scripted(vec![answer("first-actual", "LeNgTh", 0.01)]).await;
    let last = scripted(vec![answer("last-actual", "stop", 0.02)]).await;
    let outcome = CompletionLadder::new(rung(&first,"first"))
        .fallback(rung(&last,"last").unpinned())
        .truncation_retry(TruncationRetry::new(2,4096))
        .complete(CompletionRequest::new("ignored",vec![ChatMessage::user("analysis").with_image("https://example.org/image.png")])
            .max_tokens(1024).provider_options(json!({"provider":{"only":["pinned"]},"reasoning":{"effort":"low"},"usage":{"include":true}})))
        .await.expect("fallback answers");
    assert_eq!(
        outcome.response.answered_model.as_deref(),
        Some("last-actual")
    );
    assert_eq!(outcome.attempts.len(), 4);
    assert_eq!(outcome.total_usage.as_ref().unwrap().input_tokens, 40);
    assert!((outcome.total_usage.unwrap().cost_usd.unwrap() - 0.05).abs() < 1e-9);
    let requests = first.received_requests().await.unwrap();
    let caps: Vec<_> = requests
        .iter()
        .map(|r| {
            serde_json::from_slice::<Value>(&r.body).unwrap()["max_tokens"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert_eq!(caps, vec![1024, 2048, 4096]);
    let requests = last.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "last");
    assert_eq!(body["max_tokens"], 1024);
    assert!(body.get("provider").is_none());
    assert_eq!(body["reasoning"]["effort"], "low");
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
}
#[tokio::test]
async fn transport_failure_advances_but_invalid_routes_do_not() {
    let failing = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"fixture rejection"}})),
        )
        .mount(&failing)
        .await;
    let last = scripted(vec![answer("actual", "stop", 0.02)]).await;
    let request = CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")]);
    let outcome = CompletionLadder::new(rung(&failing, "first"))
        .fallback(rung(&last, "last"))
        .complete(request.clone())
        .await
        .expect("next route");
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(
        outcome.total_usage.unwrap().cost_usd,
        None,
        "failed dispatch spend is unknown"
    );
    let unsafe_rung = CompletionRung::new(
        Completer::new(Route::openai_compatible("http://example.org/v1", "fixture")),
        "first",
    );
    let error = CompletionLadder::new(unsafe_rung)
        .fallback(rung(&last, "last"))
        .complete(request)
        .await
        .expect_err("unsafe route");
    assert_eq!(error.attempts.len(), 1);
    assert_eq!(last.received_requests().await.unwrap().len(), 1);
}
#[test]
fn named_routes_and_providers_share_documented_endpoints() {
    use openhuman_embed::Provider;
    for (route, provider, url) in [
        (
            Route::openrouter("fixture"),
            Provider::openrouter("fixture"),
            "https://openrouter.ai/api/v1",
        ),
        (
            Route::moonshot("fixture"),
            Provider::moonshot("fixture"),
            "https://api.moonshot.ai/v1",
        ),
        (
            Route::minimax("fixture"),
            Provider::minimax("fixture"),
            "https://api.minimax.io/v1",
        ),
    ] {
        assert_eq!(route.base_url, url);
        assert_eq!(provider.route(), Some(&route));
    }
}

#[tokio::test]
async fn retries_stop_at_the_ceiling_and_missing_caps_never_expand() {
    let server = scripted(vec![answer("actual", "length", 0.01)]).await;
    let ladder = CompletionLadder::new(rung(&server, "requested"))
        .truncation_retry(TruncationRetry::new(255, 1500));
    let request = CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")]);
    let error = ladder
        .complete(request.clone().max_tokens(1024))
        .await
        .expect_err("bounded exhaustion");
    assert_eq!(error.attempts.len(), 2);
    assert_eq!(error.attempts[1].max_tokens, Some(1500));
    assert_eq!(error.total_usage.unwrap().cost_usd, Some(0.02));
    let error = ladder
        .complete(request)
        .await
        .expect_err("no growth without explicit cap");
    assert_eq!(error.attempts.len(), 1);
}
#[tokio::test]
async fn unpinned_rungs_are_terminal_and_success_never_tries_fallback() {
    let server = scripted(vec![answer("actual", "stop", 0.01)]).await;
    let request = CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")]);
    let error = CompletionLadder::new(rung(&server, "first").unpinned())
        .fallback(rung(&server, "next"))
        .complete(request.clone())
        .await
        .expect_err("unpinning is a last resort");
    assert!(error.attempts.is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
    let result = CompletionLadder::new(rung(&server, "first"))
        .fallback(rung(&server, "next"))
        .complete(request)
        .await
        .expect("first answers");
    assert_eq!(result.attempts.len(), 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn fallback_uses_its_own_provider_pin_and_retries_from_its_own_cap() {
    let first = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"fixture rejection"}})),
        )
        .mount(&first)
        .await;
    let fallback = scripted(vec![
        answer("actual", "length", 0.01),
        answer("actual", "stop", 0.02),
    ])
    .await;
    let outcome = CompletionLadder::new(
        rung(&first, "primary")
            .provider_options(
                json!({"provider":{"only":["primary-pin"]},"reasoning":{"effort":"low"}}),
            )
            .max_tokens(Some(16)),
    )
    .fallback(
        rung(&fallback, "fallback")
            .provider_options(
                json!({"provider":{"only":["fallback-pin"]},"reasoning":{"effort":"high"}}),
            )
            .max_tokens(Some(32)),
    )
    .truncation_retry(TruncationRetry::new(1, 128))
    .complete(
        CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")])
            .max_tokens(8)
            .provider_options(json!({"provider":{"only":["request-pin"]}})),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome
            .attempts
            .iter()
            .map(|attempt| attempt.max_tokens)
            .collect::<Vec<_>>(),
        vec![Some(16), Some(32), Some(64)]
    );
    let requests = first.received_requests().await.unwrap();
    let primary: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(primary["provider"]["only"][0], "primary-pin");
    assert_eq!(primary["max_tokens"], 16);
    let requests = fallback.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for (request, cap) in requests.iter().zip([32, 64]) {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["provider"]["only"][0], "fallback-pin");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["max_tokens"], cap);
    }
}

#[tokio::test]
async fn unpinned_rung_removes_its_own_pin_and_can_explicitly_clear_the_cap() {
    let provider = scripted(vec![answer("actual", "length", 0.01)]).await;
    let choice = rung(&provider,"model")
        .provider_options(json!({"provider":{"only":["own-pin"]},"reasoning":{"effort":"high"},"host-private":"SECRET-OPTION"}))
        .max_tokens(None).unpinned();
    assert!(!format!("{choice:?}").contains("SECRET-OPTION"));
    let error = CompletionLadder::new(choice)
        .truncation_retry(TruncationRetry::new(2, 128))
        .complete(
            CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")]).max_tokens(8),
        )
        .await
        .unwrap_err();
    assert_eq!(error.attempts.len(), 1);
    assert_eq!(error.attempts[0].max_tokens, None);
    let requests = provider.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body.get("provider").is_none());
    assert!(body.get("max_tokens").is_none());
    assert_eq!(body["reasoning"]["effort"], "high");
}

#[tokio::test]
async fn uppercase_max_tokens_retries_the_same_rung_and_counts_buyer_charges() {
    let mut truncated = answer("actual", "MAX_TOKENS", 0.0);
    truncated["usage"]["buyer_cost_micro"] = json!(3);
    let mut complete = answer("actual", "stop", 0.0);
    complete["usage"]["buyer_cost_micro"] = json!(4);
    let provider = scripted(vec![truncated, complete]).await;
    let fallback = scripted(vec![answer("unused", "stop", 0.5)]).await;
    let outcome = CompletionLadder::new(rung(&provider, "primary"))
        .fallback(rung(&fallback, "fallback"))
        .truncation_retry(TruncationRetry::new(1, 64))
        .complete(
            CompletionRequest::new("ignored", vec![ChatMessage::user("analysis")]).max_tokens(16),
        )
        .await
        .unwrap();
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(
        outcome.attempts[0].finish_reason.as_deref(),
        Some("MAX_TOKENS")
    );
    assert_eq!(outcome.attempts[1].max_tokens, Some(32));
    assert_eq!(outcome.response.usage.unwrap().cost_usd, Some(0.000004));
    assert!((outcome.total_usage.unwrap().cost_usd.unwrap() - 0.000007).abs() < 1e-12);
    assert!(fallback.received_requests().await.unwrap().is_empty());
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for (request, cap) in requests.iter().zip([16, 32]) {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["max_tokens"], cap);
    }
}

#[tokio::test]
async fn physical_request_timeout_advances_to_a_healthy_route_without_known_charge() {
    let delayed = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(answer("late", "stop", 0.01))
                .set_delay(std::time::Duration::from_millis(5_000)),
        )
        .mount(&delayed)
        .await;
    let healthy = scripted(vec![answer("healthy", "stop", 0.02)]).await;
    // Deserialization also pins the host-facing wire contract; before the
    // passthrough this field is silently ignored and the primary keeps waiting.
    let request: CompletionRequest = serde_json::from_value(json!({
        "model":"ignored", "messages":[{"role":"user","text":"analysis"}],
        "timeout_ms":50
    }))
    .unwrap();
    let ladder =
        CompletionLadder::new(rung(&delayed, "primary")).fallback(rung(&healthy, "fallback"));
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), ladder.complete(request))
        .await
        .expect("physical timeout must bound the stalled endpoint and reach fallback")
        .expect("healthy route answers");
    assert_eq!(result.attempts.len(), 2);
    assert!(result.attempts[0].failed);
    assert!(result.attempts[0].usage.is_none());
    assert_eq!(result.response.answered_model.as_deref(), Some("healthy"));
    assert_eq!(
        result.total_usage.unwrap().cost_usd,
        None,
        "timed-out spend remains unknown"
    );
    assert_eq!(delayed.received_requests().await.unwrap().len(), 1);
    assert_eq!(healthy.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn logical_deadline_and_cancellation_stop_before_fallback() {
    use openhuman_embed::{cancellation::Cancellation, CoreError};
    let delayed = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(answer("late", "stop", 0.01))
                .set_delay(std::time::Duration::from_millis(5_000)),
        )
        .mount(&delayed)
        .await;
    let healthy = scripted(vec![answer("healthy", "stop", 0.02)]).await;
    let first = Completer::new(Route::openai_compatible(
        format!("{}/v1", delayed.uri()),
        "fixture",
    ))
    .timeout(std::time::Duration::from_millis(50));
    let error = CompletionLadder::new(CompletionRung::new(first, "primary"))
        .fallback(rung(&healthy, "fallback"))
        .complete(CompletionRequest::new(
            "ignored",
            vec![ChatMessage::user("analysis")],
        ))
        .await
        .expect_err("logical deadline is terminal");
    assert!(matches!(
        error.last_error,
        CoreError::DeadlineExceeded { .. }
    ));
    assert_eq!(error.attempts.len(), 1);
    let cancelled_endpoint = MockServer::start().await;
    let cancellation = Cancellation::default();
    cancellation.cancel().await;
    let first = Completer::new(Route::openai_compatible(
        format!("{}/v1", cancelled_endpoint.uri()),
        "fixture",
    ))
    .cancellation(cancellation);
    let error = CompletionLadder::new(CompletionRung::new(first, "primary"))
        .fallback(rung(&healthy, "fallback"))
        .complete(CompletionRequest::new(
            "ignored",
            vec![ChatMessage::user("analysis")],
        ))
        .await
        .expect_err("cancellation is terminal");
    assert!(matches!(error.last_error, CoreError::Cancelled { .. }));
    assert_eq!(error.attempts.len(), 1);
    assert!(healthy.received_requests().await.unwrap().is_empty());
    assert!(cancelled_endpoint
        .received_requests()
        .await
        .unwrap()
        .is_empty());
    // A logical deadline may fire before HTTP dispatch on a loaded executor.
    assert!(delayed.received_requests().await.unwrap().len() <= 1);
}

#[tokio::test]
async fn timed_out_unknown_charge_preserves_reservation_and_budget_blocks_fallback() {
    use openhuman_embed::{
        budget::{Budget, CallBudget, ModelBudget, SpendLimits},
        CoreError,
    };
    let delayed = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(answer("late", "stop", 0.01))
                .set_delay(std::time::Duration::from_millis(5_000)),
        )
        .mount(&delayed)
        .await;
    let healthy = scripted(vec![answer("healthy", "stop", 0.02)]).await;
    let policy = ModelBudget {
        ledger: Budget::new(SpendLimits {
            tokens: None,
            cost_micros: Some(100),
        }),
        call: CallBudget {
            input_tokens: 1_000,
            output_tokens: 20,
            cost_micros: 100,
        },
    };
    let client = |server: &MockServer| {
        Completer::new(Route::openai_compatible(
            format!("{}/v1", server.uri()),
            "fixture",
        ))
        .budget(policy.clone())
    };
    let request: CompletionRequest = serde_json::from_value(json!({
        "model":"ignored", "messages":[{"role":"user","text":"analysis"}],
        "max_tokens":20,"timeout_ms":50
    }))
    .unwrap();
    let ladder = CompletionLadder::new(CompletionRung::new(client(&delayed), "primary"))
        .fallback(CompletionRung::new(client(&healthy), "fallback"));
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), ladder.complete(request))
        .await
        .expect("physical timeout is bounded")
        .expect_err("unknown charge consumes the reservation");
    assert!(matches!(error.last_error, CoreError::BudgetExceeded { .. }));
    assert_eq!(error.attempts.len(), 2);
    assert_eq!(policy.ledger.snapshot().spent.cost_micros, 100);
    assert!(healthy.received_requests().await.unwrap().is_empty());
    assert_eq!(delayed.received_requests().await.unwrap().len(), 1);
}

#[test]
fn physical_timeout_is_optional_and_round_trips_without_changing_old_requests() {
    let request: CompletionRequest = serde_json::from_value(json!({
        "model":"m", "messages":[]
    }))
    .unwrap();
    assert_eq!(request.timeout_ms, None);
    assert!(serde_json::to_value(&request)
        .unwrap()
        .get("timeout_ms")
        .is_none());
    let bounded = request.timeout_ms(120_000);
    let serialized = serde_json::to_value(&bounded).unwrap();
    assert_eq!(serialized["timeout_ms"], 120_000);
    assert_eq!(
        serde_json::from_value::<CompletionRequest>(serialized).unwrap(),
        bounded
    );
}
