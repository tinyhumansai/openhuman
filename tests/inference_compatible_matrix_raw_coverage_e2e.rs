//! Round 17 raw/E2E coverage for OpenAI/Ollama-compatible inference matrices.
//!
//! This suite uses loopback HTTP mocks and temp PATH scripts only. It must not
//! call host Ollama, MLX, Python, Piper, Whisper, or model binaries.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::local::LocalAiService;
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle as CompatibleAuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::inference::provider::traits::StreamOptions;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, Provider, ProviderDelta,
};
use openhuman_core::openhuman::tools::ToolSpec;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<(String, Option<String>, Value)>>>,
    ollama_models: Arc<Mutex<Vec<String>>>,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: validation runs this integration test with --test-threads=1.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: validation runs this integration test with --test-threads=1.
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

#[tokio::test]
async fn openai_compatible_matrix_covers_auth_requests_responses_and_streaming() {
    let (base, state) = serve_mock().await;
    let tools = vec![
        ToolSpec {
            name: "lookup".to_string(),
            description: "first definition".to_string(),
            parameters: json!({"type": "object"}),
        },
        ToolSpec {
            name: "lookup".to_string(),
            description: "duplicate definition dropped at wire boundary".to_string(),
            parameters: json!({"type": "object"}),
        },
    ];

    let provider = OpenAiCompatibleProvider::new_with_user_agent(
        "custom_openai",
        &format!("{base}/v1"),
        Some("sk-round17-secret"),
        CompatibleAuthStyle::Bearer,
        "round17-agent",
    )
    .with_temperature_unsupported_models(vec!["cold-*".to_string()])
    .with_temperature_override(Some(0.42))
    .with_openhuman_thread_id();

    let plain = provider
        .chat_with_system(Some("policy"), "hello", "plain-chat", 0.1)
        .await
        .expect("plain chat");
    assert_eq!(plain, "plain response");

    let cold = provider
        .chat_with_history(&[ChatMessage::user("omit temperature")], "cold-model", 0.9)
        .await
        .expect("temperature omission");
    assert_eq!(cold, "cold response");

    let responses = provider
        .chat_with_history(
            &[ChatMessage::system("rules"), ChatMessage::user("fallback")],
            "responses-fallback",
            0.2,
        )
        .await
        .expect("responses fallback");
    assert_eq!(responses, "responses nested text");

    let native = provider
        .chat(
            ChatRequest {
                messages: &[
                    ChatMessage::assistant(
                        json!({
                            "content": "called lookup",
                            "reasoning_content": "keep this",
                            "tool_calls": [{
                                "id": "call_prev",
                                "name": "lookup",
                                "arguments": "{\"query\":\"cached\"}"
                            }]
                        })
                        .to_string(),
                    ),
                    ChatMessage::tool(
                        json!({
                            "tool_call_id": "call_prev",
                            "content": "cached result"
                        })
                        .to_string(),
                    ),
                    ChatMessage::user("native call"),
                ],
                tools: Some(&tools),
                stream: None,
            },
            "native-tools",
            0.2,
        )
        .await
        .expect("native tool chat");
    assert_eq!(native.text.as_deref(), Some("native text"));
    assert_eq!(
        native.reasoning_content.as_deref(),
        Some("native reasoning")
    );
    assert_eq!(native.tool_calls.len(), 1);
    assert_eq!(native.tool_calls[0].name, "lookup");
    assert_eq!(native.tool_calls[0].arguments, r#"{"query":"round17"}"#);
    let usage = native.usage.expect("usage");
    assert_eq!(usage.input_tokens, 13);
    assert_eq!(usage.output_tokens, 8);
    assert_eq!(usage.cached_input_tokens, 5);
    assert!((usage.charged_amount_usd - 0.0017).abs() < f64::EPSILON);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(16);
    let streamed = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("stream")],
                tools: Some(&tools),
                stream: Some(&tx),
            },
            "stream-sse",
            0.3,
        )
        .await
        .expect("SSE stream");
    drop(tx);
    assert_eq!(streamed.text.as_deref(), Some("hello world"));
    assert_eq!(streamed.reasoning_content.as_deref(), Some("thinking"));
    assert_eq!(streamed.tool_calls.len(), 1);
    let deltas = collect_deltas(&mut rx).await;
    assert!(deltas
        .iter()
        .any(|d| matches!(d, ProviderDelta::TextDelta { delta } if delta == "hello ")));
    assert!(deltas
        .iter()
        .any(|d| matches!(d, ProviderDelta::ThinkingDelta { delta } if delta == "thinking")));
    assert!(deltas.iter().any(
        |d| matches!(d, ProviderDelta::ToolCallStart { tool_name, .. } if tool_name == "lookup")
    ));
    assert!(deltas.iter().any(
        |d| matches!(d, ProviderDelta::ToolCallArgsDelta { delta, .. } if delta.contains("stream"))
    ));

    let (json_tx, mut json_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(4);
    let json_stream = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("json stream")],
                tools: None,
                stream: Some(&json_tx),
            },
            "stream-json",
            0.3,
        )
        .await
        .expect("JSON stream fallback");
    drop(json_tx);
    assert_eq!(json_stream.text.as_deref(), Some("json stream fallback"));
    assert!(json_rx.recv().await.is_none());

    let (retry_tx, mut retry_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(8);
    let retried = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("retry without tools")],
                tools: Some(&tools),
                stream: Some(&retry_tx),
            },
            "stream-tools-unsupported",
            0.3,
        )
        .await
        .expect("stream retry strips tools");
    drop(retry_tx);
    assert_eq!(retried.text.as_deref(), Some("retry ok"));
    assert!(collect_deltas(&mut retry_rx)
        .await
        .iter()
        .any(|d| matches!(d, ProviderDelta::TextDelta { delta } if delta == "retry ok")));

    let seen = state.requests.lock().expect("requests");
    assert!(seen.iter().any(|(path, auth, body)| {
        path == "/v1/chat/completions"
            && body["model"] == "plain-chat"
            && auth.as_deref() == Some("Bearer sk-round17-secret")
    }));
    let cold_body = seen
        .iter()
        .find(|(_, _, body)| body["model"] == "cold-model")
        .expect("cold request")
        .2
        .clone();
    assert!(cold_body.get("temperature").is_none());
    let native_body = seen
        .iter()
        .find(|(_, _, body)| body["model"] == "native-tools")
        .expect("native request")
        .2
        .clone();
    assert_eq!(native_body["tools"].as_array().unwrap().len(), 1);
    assert!(native_body.get("stream_options").is_none());
    let stream_body = seen
        .iter()
        .find(|(_, _, body)| body["model"] == "stream-sse")
        .expect("stream request")
        .2
        .clone();
    assert_eq!(stream_body["stream_options"]["include_usage"], true);
    let retry_bodies: Vec<Value> = seen
        .iter()
        .filter(|(_, _, body)| body["model"] == "stream-tools-unsupported")
        .map(|(_, _, body)| body.clone())
        .collect();
    assert_eq!(retry_bodies.len(), 2);
    assert!(retry_bodies[0].get("tools").is_some());
    assert!(retry_bodies[1].get("tools").is_none());
}

#[tokio::test]
async fn compatible_error_matrix_covers_status_malformed_and_no_fallback_paths() {
    let (base, _state) = serve_mock().await;
    let provider = OpenAiCompatibleProvider::new(
        "custom_openai",
        &format!("{base}/v1"),
        Some("sk-should-redact"),
        CompatibleAuthStyle::Bearer,
    );

    let malformed = provider
        .chat_with_system(None, "bad json", "malformed-chat-json", 0.1)
        .await
        .expect_err("malformed chat response");
    assert!(malformed
        .to_string()
        .contains("unexpected chat-completions payload"));
    assert!(!malformed.to_string().contains("sk-should-redact"));

    let empty = provider
        .chat_with_history(&[ChatMessage::user("empty")], "empty-choices", 0.1)
        .await
        .expect_err("empty choices");
    assert!(empty.to_string().contains("No response"));

    let denied = provider
        .chat_with_system(None, "denied", "policy-denied", 0.1)
        .await
        .expect_err("403 denied");
    assert!(denied.to_string().contains("access denied"));
    assert!(!denied.to_string().contains("sk-should-redact"));

    let responses_status = provider
        .chat_with_history(
            &[ChatMessage::user("fallback")],
            "responses-status-error",
            0.1,
        )
        .await
        .expect_err("responses status error");
    assert!(responses_status.to_string().contains("Responses API error"));
    assert!(!responses_status.to_string().contains("sk-should-redact"));

    let responses_malformed = provider
        .chat_with_history(&[ChatMessage::user("fallback")], "responses-malformed", 0.1)
        .await
        .expect_err("responses malformed");
    assert!(responses_malformed
        .to_string()
        .contains("unexpected payload"));

    let no_fallback = OpenAiCompatibleProvider::new_no_responses_fallback(
        "glm",
        &format!("{base}/v1"),
        None,
        CompatibleAuthStyle::None,
    );
    let not_found = no_fallback
        .chat_with_system(None, "missing", "missing-no-fallback", 0.1)
        .await
        .expect_err("no responses fallback");
    assert!(not_found.to_string().contains("endpoint URL"));

    let (sse_tx, mut sse_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(4);
    let streaming_status = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("stream fail")],
                tools: None,
                stream: Some(&sse_tx),
            },
            "stream-status-error",
            0.1,
        )
        .await
        .expect_err("stream and non-stream fallback both fail");
    drop(sse_tx);
    assert!(streaming_status.to_string().contains("stream failed"));
    assert!(sse_rx.recv().await.is_none());

    let mut raw_stream = provider.stream_chat_with_system(
        None,
        "raw stream",
        "raw-stream-invalid-json",
        0.1,
        StreamOptions::new(true),
    );
    let first = raw_stream.next().await.expect("first raw stream chunk");
    assert!(first
        .expect_err("invalid SSE JSON")
        .to_string()
        .contains("JSON"));

    let mut http_stream = provider.stream_chat_with_system(
        None,
        "raw stream",
        "raw-stream-http-error",
        0.1,
        StreamOptions::new(true),
    );
    let first = http_stream.next().await.expect("HTTP error chunk");
    assert!(!first.expect_err("HTTP stream error").to_string().is_empty());
}

#[tokio::test]
async fn ollama_compatible_matrix_covers_authless_chat_and_streaming_errors() {
    let (base, state) = serve_mock().await;
    let provider = OpenAiCompatibleProvider::new(
        "ollama",
        &format!("{base}/ollama/v1"),
        None,
        CompatibleAuthStyle::None,
    );

    let chat = provider
        .chat_with_system(Some("ollama policy"), "hello", "ollama-chat", 0.0)
        .await
        .expect("ollama-compatible chat");
    assert_eq!(chat, "ollama compatible response");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(8);
    let streamed = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("ollama stream")],
                tools: None,
                stream: Some(&tx),
            },
            "ollama-stream",
            0.0,
        )
        .await
        .expect("ollama-compatible stream");
    drop(tx);
    assert_eq!(streamed.text.as_deref(), Some("ollama stream"));
    assert!(collect_deltas(&mut rx)
        .await
        .iter()
        .any(|d| matches!(d, ProviderDelta::TextDelta { delta } if delta == "ollama stream")));

    let malformed = provider
        .chat_with_history(&[ChatMessage::user("bad")], "ollama-malformed", 0.0)
        .await
        .expect_err("ollama malformed response");
    assert!(malformed
        .to_string()
        .contains("unexpected chat-completions payload"));

    let seen = state.requests.lock().expect("requests");
    assert!(seen.iter().any(|(path, auth, body)| {
        path == "/ollama/v1/chat/completions" && auth.is_none() && body["model"] == "ollama-chat"
    }));
}

#[tokio::test]
async fn ollama_admin_matrix_covers_list_show_pull_failure_branches() {
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "gemma4:e4b-it-q8_0".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.vision_model_id = "vision-missing".to_string();
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.preload_embedding_model = true;
    config.local_ai.preload_vision_model = true;

    let scripts = tempdir().expect("scripts");
    write_stub_script(scripts.path(), "ollama", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "mlx_lm.generate", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "piper", "#!/bin/sh\nexit 42\n");
    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _ollama_bin = EnvVarGuard::unset("OLLAMA_BIN");
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _whisper_bin = EnvVarGuard::unset("WHISPER_BIN");

    let service = LocalAiService::new(&config);
    let diagnostics = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diagnostics["ollama_running"], true);
    assert_eq!(diagnostics["expected"]["chat_found"], true);
    assert_eq!(diagnostics["expected"]["embedding_found"], true);
    assert_eq!(diagnostics["expected"]["vision_found"], false);
    assert!(diagnostics["installed_models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["name"] == "bge-m3" && model["context_length"] == 1024));

    let mut tags_500 = config.clone();
    tags_500.local_ai.base_url = Some(format!("{base}/tags-500"));
    let tags_report = service.diagnostics(&tags_500).await.expect("tags 500");
    assert_eq!(tags_report["ollama_running"], false);
    assert!(tags_report["issues"][0]
        .as_str()
        .unwrap()
        .contains("not running or not reachable"));

    let mut tags_bad_json = config.clone();
    tags_bad_json.local_ai.base_url = Some(format!("{base}/tags-bad-json"));
    let tags_bad_report = service
        .diagnostics(&tags_bad_json)
        .await
        .expect("tags bad json");
    assert_eq!(tags_bad_report["ollama_running"], true);
    assert!(tags_bad_report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue.as_str().unwrap().contains("Failed to list models")));

    let mut pull_config = config.clone();
    pull_config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    let pull_error = service
        .download_asset(&pull_config, "chat")
        .await
        .expect_err("pull failure");
    assert!(pull_error.contains("ollama pull failed with status 500"));
}

async fn collect_deltas(rx: &mut tokio::sync::mpsc::Receiver<ProviderDelta>) -> Vec<ProviderDelta> {
    let mut out = Vec::new();
    while let Some(delta) = rx.recv().await {
        out.push(delta);
    }
    out
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    *state.ollama_models.lock().expect("models") =
        vec!["gemma4:e4b-it-q8_0".to_string(), "bge-m3".to_string()];
    let app = Router::new()
        .route("/v1/chat/completions", post(openai_chat_completions))
        .route("/v1/responses", post(openai_responses))
        .route(
            "/ollama/v1/chat/completions",
            post(ollama_compatible_chat_completions),
        )
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/pull", post(ollama_pull))
        .route("/tags-500/api/tags", get(ollama_tags_500))
        .route("/tags-bad-json/api/tags", get(ollama_tags_bad_json))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    (format!("http://{addr}"), state)
}

async fn openai_chat_completions(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/chat/completions", &headers, body.clone());
    let model = body["model"].as_str().unwrap_or_default();
    match model {
        "plain-chat" => Json(json!({
            "choices": [{ "message": { "content": "plain response" } }]
        }))
        .into_response(),
        "cold-model" => Json(json!({
            "choices": [{ "message": { "content": "cold response" } }]
        }))
        .into_response(),
        "responses-fallback" | "responses-status-error" | "responses-malformed" => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"message": "chat route missing sk-chat-secret"}})),
        )
            .into_response(),
        "native-tools" => Json(json!({
            "choices": [{
                "message": {
                    "content": "native text",
                    "reasoning_content": " native reasoning ",
                    "tool_calls": [{
                        "id": "call_round17",
                        "type": "function",
                        "function": {
                            "name": "lookup",
                            "arguments": { "query": "round17" }
                        }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 21,
                "completion_tokens": 9,
                "total_tokens": 30,
                "prompt_tokens_details": { "cached_tokens": 2 }
            },
            "openhuman": {
                "usage": {
                    "input_tokens": 13,
                    "output_tokens": 8,
                    "cached_input_tokens": 5
                },
                "billing": { "charged_amount_usd": 0.0017 }
            }
        }))
        .into_response(),
        "stream-sse" => sse_response(
            [
                json!({"choices":[{"delta":{"content":"hello "}}]}),
                json!({"choices":[{"delta":{"reasoning_content":"thinking"}}]}),
                json!({"choices":[{"delta":{"content":"world","tool_calls":[{
                    "index": 0,
                    "id": "call_stream",
                    "type": "function",
                    "function": {"name": "lookup", "arguments": "{\"query\":\"stream\"}"}
                }]}}]}),
                json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}),
            ],
            true,
        ),
        "stream-json" => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "choices": [{ "message": { "content": "json stream fallback" } }]
                })
                .to_string(),
            ))
            .expect("json stream")
            .into_response(),
        "stream-tools-unsupported" if body.get("tools").is_some() => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"message":"model does not support tools"}})),
        )
            .into_response(),
        "stream-tools-unsupported" => sse_response(
            [json!({"choices":[{"delta":{"content":"retry ok"}}]})],
            true,
        ),
        "stream-status-error" => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":{"message":"stream failed sk-stream-secret"}})),
        )
            .into_response(),
        "raw-stream-invalid-json" => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from("data: {not-json}\n\n"))
            .expect("bad sse")
            .into_response(),
        "raw-stream-http-error" => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error":{"message":"bad gateway"}})),
        )
            .into_response(),
        "malformed-chat-json" | "ollama-malformed" => {
            Json(json!({"choices": "wrong"})).into_response()
        }
        "empty-choices" => Json(json!({"choices": []})).into_response(),
        "policy-denied" => (
            StatusCode::FORBIDDEN,
            Json(json!({"error":{"message":"access denied sk-policy-secret"}})),
        )
            .into_response(),
        "missing-no-fallback" => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":{"message":"missing model"}})),
        )
            .into_response(),
        _ => Json(json!({
            "choices": [{ "message": { "content": "fallback response" } }]
        }))
        .into_response(),
    }
}

async fn openai_responses(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/responses", &headers, body.clone());
    match body["model"].as_str().unwrap_or_default() {
        "responses-status-error" => (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({"error":{"message":"budget exhausted sk-responses-secret"}})),
        )
            .into_response(),
        "responses-malformed" => Json(json!({"output_text": 123})).into_response(),
        _ => Json(json!({
            "output": [{
                "content": [{ "type": "output_text", "text": "responses nested text" }]
            }]
        }))
        .into_response(),
    }
}

async fn ollama_compatible_chat_completions(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(
        &state,
        "/ollama/v1/chat/completions",
        &headers,
        body.clone(),
    );
    match body["model"].as_str().unwrap_or_default() {
        "ollama-chat" => Json(json!({
            "choices": [{ "message": { "content": "ollama compatible response" } }]
        }))
        .into_response(),
        "ollama-stream" => sse_response(
            [json!({"choices":[{"delta":{"content":"ollama stream"}}]})],
            true,
        ),
        "ollama-malformed" => Json(json!({"choices": "wrong"})).into_response(),
        _ => Json(json!({
            "choices": [{ "message": { "content": "ollama fallback" } }]
        }))
        .into_response(),
    }
}

async fn ollama_tags(State(state): State<MockState>) -> impl IntoResponse {
    let models = state
        .ollama_models
        .lock()
        .expect("models")
        .iter()
        .map(|name| json!({ "name": name, "model": name }))
        .collect::<Vec<_>>();
    Json(json!({ "models": models })).into_response()
}

async fn ollama_show(Json(body): Json<Value>) -> impl IntoResponse {
    let model = body["model"].as_str().unwrap_or_default();
    match model {
        "gemma4:e4b-it-q8_0" => Json(json!({
            "model_info": {
                "general.context_length": 8192,
                "llama.context_length": 8192
            }
        }))
        .into_response(),
        "bge-m3" => Json(json!({
            "model_info": {
                "general.context_length": 1024,
                "llama.context_length": 1024
            }
        }))
        .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "model not found"})),
        )
            .into_response(),
    }
}

async fn ollama_pull(Json(body): Json<Value>) -> impl IntoResponse {
    let name = body["name"].as_str().unwrap_or_default();
    if name == "vision-missing" || name == "gemma3:1b-it-qat" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "pull denied"})),
        )
            .into_response();
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from(
            [
                json!({"status":"pulling manifest"}).to_string(),
                json!({"status":"success"}).to_string(),
            ]
            .join("\n")
                + "\n",
        ))
        .expect("pull")
        .into_response()
}

async fn ollama_tags_500() -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "tags failed").into_response()
}

async fn ollama_tags_bad_json() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not-json"))
        .expect("bad tags")
}

fn sse_response<const N: usize>(events: [Value; N], done: bool) -> axum::response::Response {
    let mut body = String::new();
    for event in events {
        body.push_str("data: ");
        body.push_str(&event.to_string());
        body.push_str("\n\n");
    }
    if done {
        body.push_str("data: [DONE]\n\n");
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .expect("sse")
        .into_response()
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state
        .requests
        .lock()
        .expect("requests")
        .push((path.to_string(), auth_header(headers), body));
}

fn auth_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .or_else(|| headers.get("x-api-key"))
        .or_else(|| headers.get("x-custom-auth"))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn temp_config(tmp: &TempDir) -> Config {
    let root = tmp.path().join(".openhuman");
    std::fs::create_dir_all(root.join("workspace")).expect("workspace dir");
    let mut config = Config::default();
    config.config_path = root.join("config.toml");
    config.workspace_dir = root.join("workspace");
    config.secrets.encrypt = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config
}

fn write_stub_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    path
}
