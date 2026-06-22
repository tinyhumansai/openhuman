//! Round 26 raw/E2E coverage for high-yield inference cold paths.
//!
//! This suite uses loopback HTTP mocks and temporary fake executables only. It
//! must not call host Ollama, MLX, Python, whisper, piper, local AI binaries,
//! models, or downloads.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::local::LocalAiService;
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle as CompatibleAuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::inference::provider::traits::{ChatRequest, ProviderDelta};
use openhuman_core::openhuman::inference::provider::{ChatMessage, Provider};
use openhuman_core::openhuman::tools::ToolSpec;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<SeenRequest>>>,
    tool_retry_attempts: Arc<Mutex<usize>>,
}

#[derive(Clone, Debug)]
struct SeenRequest {
    path: String,
    auth: Option<String>,
    body: Value,
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
async fn compatible_streaming_covers_tool_deltas_json_fallback_and_retry_without_tools() {
    let (base, state) = serve_mock().await;
    let provider = OpenAiCompatibleProvider::new(
        "round26-compatible",
        &format!("{base}/v1"),
        Some("sk-round26"),
        CompatibleAuthStyle::Bearer,
    );

    let tools = vec![
        tool_spec("lookup"),
        tool_spec("lookup"),
        tool_spec("summarize"),
    ];
    let messages = vec![
        ChatMessage::tool(json!({"tool_call_id":"orphan","content":"drop me"}).to_string()),
        ChatMessage::assistant(
            json!({
                "content": "prior",
                "reasoning_content": "keep-thinking",
                "tool_calls": [
                    {"id":"answered","name":"lookup","arguments":"{\"q\":\"old\"}"},
                    {"id":"dangling","name":"summarize","arguments":"{}"}
                ]
            })
            .to_string(),
        ),
        ChatMessage::tool(json!({"tool_call_id":"answered","content":"old answer"}).to_string()),
        ChatMessage::user("stream with tools"),
    ];
    let (delta_tx, mut delta_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(16);
    let streamed = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: Some(&tools),
                stream: Some(&delta_tx),
                max_tokens: None,
            },
            "stream-tools",
            0.4,
        )
        .await
        .expect("streaming native chat");
    drop(delta_tx);

    assert_eq!(streamed.text.as_deref(), Some("hello world"));
    assert_eq!(streamed.reasoning_content.as_deref(), Some("think more"));
    assert_eq!(streamed.tool_calls.len(), 1);
    assert_eq!(streamed.tool_calls[0].id, "call_round26");
    assert_eq!(streamed.tool_calls[0].name, "lookup");
    assert_eq!(streamed.tool_calls[0].arguments, "{\"q\":\"rust\"}");
    let mut deltas = Vec::new();
    while let Some(delta) = delta_rx.recv().await {
        deltas.push(delta);
    }
    assert!(deltas
        .iter()
        .any(|delta| matches!(delta, ProviderDelta::TextDelta { delta } if delta == "hello ")));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ProviderDelta::ThinkingDelta { delta } if delta == "think "
    )));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ProviderDelta::ToolCallStart { call_id, tool_name }
            if call_id == "call_round26" && tool_name == "lookup"
    )));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ProviderDelta::ToolCallArgsDelta { call_id, delta }
            if call_id == "call_round26" && delta.contains("rust")
    )));

    let (json_tx, _json_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(4);
    let json_fallback = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("json stream fallback")],
                tools: None,
                stream: Some(&json_tx),
                max_tokens: None,
            },
            "json-stream",
            0.2,
        )
        .await
        .expect("non-SSE stream falls back to JSON parse");
    assert_eq!(json_fallback.text.as_deref(), Some("json fallback ok"));
    assert_eq!(json_fallback.usage.unwrap().cached_input_tokens, 3);

    let (retry_tx, mut retry_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(8);
    let retry = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("retry without tools")],
                tools: Some(&[tool_spec("lookup")]),
                stream: Some(&retry_tx),
                max_tokens: None,
            },
            "tool-retry",
            0.2,
        )
        .await
        .expect("tool schema rejection retries streaming without tools");
    drop(retry_tx);
    assert_eq!(retry.text.as_deref(), Some("retried ok"));
    assert_eq!(*state.tool_retry_attempts.lock().expect("attempts"), 2);
    assert!(matches!(
        retry_rx.recv().await,
        Some(ProviderDelta::TextDelta { delta }) if delta == "retried ok"
    ));

    let seen = state.requests.lock().expect("requests");
    let stream_body = seen
        .iter()
        .find(|req| req.body["model"] == "stream-tools")
        .expect("stream request body");
    assert_eq!(stream_body.auth.as_deref(), Some("Bearer sk-round26"));
    assert_eq!(stream_body.path, "/v1/chat/completions");
    assert_eq!(stream_body.body["stream"], true);
    assert_eq!(stream_body.body["stream_options"]["include_usage"], true);
    assert_eq!(stream_body.body["tools"].as_array().unwrap().len(), 2);
    let wire_messages = stream_body.body["messages"].as_array().unwrap();
    assert_ne!(wire_messages[0]["role"], "tool");
    let assistant = wire_messages
        .iter()
        .find(|msg| msg["role"] == "assistant")
        .expect("assistant message");
    assert_eq!(assistant["tool_calls"].as_array().unwrap().len(), 1);
    assert_eq!(assistant["reasoning_content"], "keep-thinking");
}

#[tokio::test]
async fn local_service_covers_mocked_bootstrap_assets_diagnostics_and_embed() {
    let tmp = tempdir().expect("tempdir");
    let fake_bin_dir = tmp.path().join("fake-bin");
    std::fs::create_dir_all(&fake_bin_dir).expect("fake bin dir");
    let fake_ollama = write_stub_script(&fake_bin_dir, "ollama", "#!/bin/sh\nexit 0\n");
    let fake_mlx = write_stub_script(&fake_bin_dir, "mlx_lm", "#!/bin/sh\nexit 0\n");
    let _ollama_bin = EnvVarGuard::set("OLLAMA_BIN", &fake_ollama);
    let _mlx_bin = EnvVarGuard::set("MLX_LM_BIN", &fake_mlx);
    let _python = EnvVarGuard::unset("PYTHON");
    let _whisper = EnvVarGuard::unset("WHISPER_BIN");
    let _piper = EnvVarGuard::unset("PIPER_BIN");

    let (base, state) = serve_mock().await;
    let _ollama_url = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.provider = "ollama".to_string();
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.preload_embedding_model = true;
    config.local_ai.preload_stt_model = false;
    config.local_ai.preload_tts_voice = false;
    config.local_ai.stt_download_url = Some(format!("{base}/asset/stt"));
    config.local_ai.tts_download_url = Some(format!("{base}/asset/tts"));

    let service = LocalAiService::new(&config);
    service.bootstrap(&config).await;
    let status = service.status();
    assert_eq!(status.state, "ready");
    assert_eq!(status.embedding_state, "ready");
    assert_eq!(status.provider, "ollama");
    assert_eq!(
        status.model_path.as_deref(),
        Some("ollama://gemma3:1b-it-qat")
    );

    let assets = service.assets_status(&config).await.expect("assets status");
    assert_eq!(assets.chat.state, "ready");
    assert_eq!(assets.embedding.state, "ready");
    assert_eq!(assets.vision.state, "disabled");
    assert_eq!(assets.stt.state, "ondemand");
    assert!(
        matches!(assets.tts.state.as_str(), "ondemand" | "ready"),
        "tts state should be on-demand or already resolved, got {}",
        assets.tts.state
    );
    assert!(assets.ollama_available);

    let progress = service
        .downloads_progress(&config)
        .await
        .expect("downloads progress");
    assert_eq!(progress.chat.state, "ready");
    assert_eq!(progress.embedding.state, "ready");
    assert_eq!(progress.stt.state, "ondemand");

    let diagnostics = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diagnostics["ollama_running"], true);
    assert_eq!(diagnostics["expected"]["chat_found"], true);
    assert_eq!(diagnostics["expected"]["embedding_found"], true);
    assert!(diagnostics["ollama_binary_path"]
        .as_str()
        .unwrap()
        .contains("ollama"));
    assert_eq!(
        diagnostics["installed_models"]
            .as_array()
            .expect("installed models")
            .len(),
        2
    );

    let embedded = service
        .embed(
            &config,
            &[
                "  first input  ".to_string(),
                "".to_string(),
                "second input".to_string(),
            ],
        )
        .await
        .expect("mocked embed");
    assert_eq!(embedded.model_id, "bge-m3");
    assert_eq!(embedded.dimensions, 3);
    assert_eq!(embedded.vectors.len(), 2);

    let seen = state.requests.lock().expect("requests");
    assert!(seen.iter().any(|req| req.path == "/api/show"
        && req
            .body
            .get("name")
            .or_else(|| req.body.get("model"))
            .and_then(Value::as_str)
            == Some("___nonexistent_probe___")));
    let embed = seen
        .iter()
        .find(|req| req.path == "/api/embed")
        .expect("embed request");
    assert_eq!(embed.body["model"], "bge-m3");
    assert_eq!(embed.body["input"], json!(["first input", "second input"]));
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/pull", post(ollama_pull))
        .route("/api/embed", post(ollama_embed))
        .route("/asset/stt", get(asset_bytes))
        .route("/asset/tts", get(asset_bytes))
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

async fn chat_completions(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/chat/completions", &headers, body.clone());
    match body["model"].as_str().unwrap_or_default() {
        "stream-tools" => sse_response(
            [
                r#"data: {"choices":[{"delta":{"content":"hello ","reasoning_content":"think ","tool_calls":[{"index":0,"id":"call_round26","function":{"name":"lookup","arguments":"{\"q\""}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"world","reasoning_content":"more","tool_calls":[{"index":0,"function":{"arguments":":\"rust\"}"}}]}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"openhuman":{"usage":{"input_tokens":11,"output_tokens":6,"cached_input_tokens":4},"billing":{"charged_amount_usd":0.01}}}"#,
                "data: [DONE]",
            ]
            .join("\n\n"),
        )
        .into_response(),
        "json-stream" => (
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({
                "choices": [{"message": {"content": "json fallback ok"}}],
                "usage": {
                    "prompt_tokens": 8,
                    "completion_tokens": 4,
                    "total_tokens": 12,
                    "prompt_tokens_details": {"cached_tokens": 3}
                }
            })),
        )
            .into_response(),
        "tool-retry" => {
            let mut attempts = state.tool_retry_attempts.lock().expect("attempts");
            *attempts += 1;
            if body.get("tools").is_some() {
                (
                    StatusCode::BAD_REQUEST,
                    "this model does not support tools or tool_choice",
                )
                    .into_response()
            } else {
                sse_response(
                    [
                        r#"data: {"choices":[{"delta":{"content":"retried ok"}}]}"#,
                        "data: [DONE]",
                    ]
                    .join("\n\n"),
                )
                .into_response()
            }
        }
        _ => Json(json!({"choices":[{"message":{"content":"ok"}}]})).into_response(),
    }
}

async fn ollama_tags(State(state): State<MockState>, headers: HeaderMap) -> impl IntoResponse {
    remember(&state, "/api/tags", &headers, json!({}));
    Json(json!({
        "models": [
            {
                "name": "gemma3:1b-it-qat",
                "model": "gemma3:1b-it-qat",
                "modified_at": "2026-05-30T00:00:00Z",
                "size": 1234,
                "digest": "sha256:chat"
            },
            {
                "name": "bge-m3:latest",
                "model": "bge-m3:latest",
                "modified_at": "2026-05-30T00:00:00Z",
                "size": 5678,
                "digest": "sha256:embed"
            }
        ]
    }))
}

async fn ollama_show(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/api/show", &headers, body.clone());
    let model = body
        .get("model")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if model == "___nonexistent_probe___" {
        return (StatusCode::NOT_FOUND, "model not found").into_response();
    }
    Json(json!({
        "model_info": {
            "llama.context_length": if model.starts_with("bge-m3") { 8192 } else { 4096 }
        }
    }))
    .into_response()
}

async fn ollama_pull(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/api/pull", &headers, body);
    sse_response(
        [
            r#"{"status":"downloading","digest":"sha256:a","total":100,"completed":100}"#,
            r#"{"status":"success"}"#,
        ]
        .join("\n"),
    )
}

async fn ollama_embed(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/api/embed", &headers, body);
    Json(json!({
        "model": "bge-m3",
        "embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]
    }))
}

async fn asset_bytes() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(vec![7u8; 1024]))
        .expect("asset response")
}

fn sse_response(body: String) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .expect("sse response")
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state.requests.lock().expect("requests").push(SeenRequest {
        path: path.to_string(),
        auth: auth_header(headers),
        body,
    });
}

fn auth_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .or_else(|| headers.get("x-api-key"))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn tool_spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: format!("{name} tool"),
        parameters: json!({
            "type": "object",
            "properties": {
                "q": {"type": "string"}
            }
        }),
    }
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
    std::fs::write(&path, body).expect("write stub script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    path
}
