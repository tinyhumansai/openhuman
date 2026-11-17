//! Managed-model SSE proxy replay and empty-response recovery.

use super::*;

async fn spawn_sse_chat_server(chunks: Vec<Value>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind SSE proxy");
    let addr = listener.local_addr().expect("SSE proxy address");
    let body = chunks
        .into_iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n";
    let app = axum::Router::new()
        .route(
            "/openai/v1/chat/completions",
            axum::routing::post(
                |axum::extract::State(body): axum::extract::State<String>| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                        body,
                    )
                },
            ),
        )
        .with_state(body);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve SSE proxy");
    });
    addr.to_string()
}

async fn spawn_recovering_sse_proxy(
    first_delta: Value,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct ProxyState {
        first_delta: Value,
        calls: std::sync::Arc<AtomicUsize>,
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind recovering SSE proxy");
    let addr = listener.local_addr().expect("recovering proxy address");
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new()
        .route(
            "/openai/v1/chat/completions",
            axum::routing::post(
                |axum::extract::State(state): axum::extract::State<ProxyState>| async move {
                    let attempt = state.calls.fetch_add(1, Ordering::SeqCst);
                    let delta = if attempt == 0 {
                        state.first_delta
                    } else {
                        serde_json::json!({ "content": "Hello there" })
                    };
                    let body = format!(
                        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                        serde_json::json!({ "choices": [{ "index": 0, "delta": delta }] }),
                        serde_json::json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
                    );
                    ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], body)
                },
            ),
        )
        .with_state(ProxyState {
            first_delta,
            calls: calls.clone(),
        });
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve recovering SSE proxy");
    });
    (addr.to_string(), calls)
}

#[tokio::test]
async fn managed_stream_proxy_distinguishes_visible_and_reasoning_only_content() {
    use futures::StreamExt;
    use tinyinference_llm::model::StreamAccumulator;

    for (delta, expected_text) in [
        (
            serde_json::json!({ "content": "Hello there" }),
            "Hello there",
        ),
        (serde_json::json!({ "reasoning": "Hello there" }), ""),
        (serde_json::json!({ "content": "<think>Hello there" }), ""),
    ] {
        let tmp = tempfile::TempDir::new().expect("scratch credentials");
        seed_app_session(tmp.path());
        let addr = spawn_sse_chat_server(vec![
            serde_json::json!({ "choices": [{ "index": 0, "delta": delta }] }),
            serde_json::json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
        ])
        .await;
        let backend = backend_pointed_at(&addr, tmp.path());
        let stream = backend
            .stream(&(), ModelRequest::new(vec![Message::user("hi")]))
            .await
            .expect("mock SSE response");
        let mut accumulated = StreamAccumulator::new();
        for item in stream.collect::<Vec<_>>().await {
            accumulated.push(&item);
        }
        let response = accumulated.finish().expect("completed SSE stream");
        assert_eq!(response.text(), expected_text);
        assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    }
}

#[tokio::test]
async fn managed_stream_retries_a_proxy_completion_without_visible_text() {
    use std::sync::atomic::Ordering;
    use tinyagents_harness::context::RunConfig;
    use tinyagents_harness::runtime::{AgentHarness, RunPolicy};

    for first_delta in [
        serde_json::json!({ "reasoning": "Hello there" }),
        serde_json::json!({ "content": "<think>Hello there" }),
    ] {
        let tmp = tempfile::TempDir::new().expect("scratch credentials");
        seed_app_session(tmp.path());
        let (addr, calls) = spawn_recovering_sse_proxy(first_delta).await;
        let mut harness: AgentHarness<()> = AgentHarness::new();
        harness.with_model(std::sync::Arc::new(backend_pointed_at(&addr, tmp.path())));
        harness.with_policy(RunPolicy {
            empty_response_retries: 1,
            ..RunPolicy::default()
        });

        let run = harness
            .invoke_streaming(
                &(),
                (),
                RunConfig::new("proxy-retry"),
                vec![Message::user("hi")],
            )
            .await
            .expect("second proxy completion should produce a visible answer");
        assert_eq!(run.text(), Some("Hello there".to_string()));
        assert_eq!(run.model_calls, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

/// Serves one SSE completion and records the JSON body it was sent.
async fn spawn_capturing_sse_server() -> (String, std::sync::Arc<std::sync::Mutex<Option<Value>>>) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind capturing server");
    let addr = listener.local_addr().expect("capturing server address");
    let app = axum::Router::new()
        .route(
            "/openai/v1/chat/completions",
            axum::routing::post(
                |axum::extract::State(seen): axum::extract::State<
                    std::sync::Arc<std::sync::Mutex<Option<Value>>>,
                >,
                 axum::Json(body): axum::Json<Value>| async move {
                    *seen.lock().unwrap() = Some(body);
                    let chunk = serde_json::json!({
                        "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
                    });
                    (
                        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                        format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                    )
                },
            ),
        )
        .with_state(seen.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve capturing");
    });
    (addr.to_string(), seen)
}

/// Anthropic's prompt cache is opt-in: the managed model must send
/// `cache_control` breakpoints for a Claude upstream and nothing extra for any
/// other, otherwise every call re-bills the whole stable prefix.
#[tokio::test]
async fn managed_stream_marks_the_cacheable_prefix_for_anthropic_models_only() {
    use futures::StreamExt;
    use tinyinference_llm::model::{PromptSegment, SegmentRole};

    for (model, expect_markers) in [
        ("anthropic/claude-sonnet-4-6", true),
        ("openrouter/anthropic/claude-haiku-4.5", true),
        ("deepseek/deepseek-v4-flash", false),
    ] {
        let tmp = tempfile::TempDir::new().expect("scratch credentials");
        seed_app_session(tmp.path());
        let (addr, seen) = spawn_capturing_sse_server().await;
        let backend = backend_pointed_at(&addr, tmp.path());
        let request = ModelRequest::new(vec![Message::system("stable rules"), Message::user("hi")])
            .with_model(model)
            .with_cache_segments(vec![PromptSegment {
                id: "system".into(),
                role: SegmentRole::System,
                cacheable: true,
            }]);
        let stream = backend
            .stream(&(), request)
            .await
            .expect("mock SSE response");
        let _ = stream.collect::<Vec<_>>().await;
        let body = seen.lock().unwrap().clone().expect("request body captured");
        let markers = body.to_string().matches("cache_control").count();
        assert_eq!(markers > 0, expect_markers, "model={model} body={body}");
    }
}

/// Streams `chunks` from a mock backend and returns the terminal response.
async fn streamed_response(chunks: Vec<Value>) -> tinyinference_llm::model::ModelResponse {
    use futures::StreamExt;
    use tinyinference_llm::model::ModelStreamItem;

    let tmp = tempfile::TempDir::new().expect("scratch credentials");
    seed_app_session(tmp.path());
    let addr = spawn_sse_chat_server(chunks).await;
    let backend = backend_pointed_at(&addr, tmp.path());
    backend
        .stream(&(), ModelRequest::new(vec![Message::user("hi")]))
        .await
        .expect("mock SSE response")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .find_map(|item| match item {
            ModelStreamItem::Completed(response) => Some(response),
            _ => None,
        })
        .expect("stream completes")
}

/// The backend bills a streamed call and reports the charge on its own frame
/// before `[DONE]`. The streamed response must carry that charge to the cost
/// accounting, as the non-streaming path always has; it used to be dropped,
/// so every streamed managed turn fell back to a guessed rate.
#[tokio::test]
async fn managed_stream_carries_the_backend_charge_to_the_cost_accounting() {
    let response = streamed_response(vec![
        serde_json::json!({ "choices": [{ "index": 0, "delta": { "content": "hi" }, "finish_reason": "stop" }] }),
        serde_json::json!({ "choices": [], "usage": { "prompt_tokens": 1000, "completion_tokens": 20, "total_tokens": 1020 } }),
        serde_json::json!({ "openhuman": {
            "usage": { "input_tokens": 1000, "output_tokens": 20, "total_tokens": 1020, "cached_input_tokens": 800 },
            "billing": { "charged_amount_usd": 0.0000634 }
        } }),
    ])
    .await;

    let billed = tinyinference_llm::usage::recover_usage(&response).expect("usage reported");
    assert!(
        billed.charged_amount_usd.is_some(),
        "the backend's charge reached the cost accounting"
    );
    assert!((billed.charged_amount_usd.unwrap() - 0.0000634).abs() < 1e-12);
    assert_eq!(billed.usage.cache_read_tokens, 800);
}

/// A free route bills $0, and that is a known cost, not a missing one.
#[tokio::test]
async fn managed_stream_keeps_a_zero_charge_as_known() {
    let response = streamed_response(vec![
        serde_json::json!({ "choices": [{ "index": 0, "delta": { "content": "hi" }, "finish_reason": "stop" }] }),
        serde_json::json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12 } }),
        serde_json::json!({ "openhuman": { "billing": { "charged_amount_usd": 0.0 } } }),
    ])
    .await;
    let billed = tinyinference_llm::usage::recover_usage(&response).expect("usage reported");
    assert_eq!(billed.charged_amount_usd, Some(0.0));
}

/// A stream with no billing frame carries no charge: the cost stays unknown
/// for an uncatalogued model instead of being priced at a default rate.
#[tokio::test]
async fn managed_stream_without_a_billing_frame_has_no_charge() {
    let response = streamed_response(vec![
        serde_json::json!({ "choices": [{ "index": 0, "delta": { "content": "hi" }, "finish_reason": "stop" }] }),
        serde_json::json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12 } }),
    ])
    .await;
    let billed = tinyinference_llm::usage::recover_usage(&response).expect("usage reported");
    assert_eq!(billed.charged_amount_usd, None);
}
