//! Round 15 raw/E2E coverage for inference provider and local runtime paths.
//!
//! These tests use only loopback HTTP mocks and temp workspaces. They do not
//! require real Ollama, LM Studio, Piper, Whisper, Python, or model binaries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::openhuman::config::schema::cloud_providers::{
    AuthStyle as CloudAuthStyle, CloudProviderCreds,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::inference::local::LocalAiService;
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle as CompatibleAuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::inference::provider::factory::{
    auth_key_for_slug, create_chat_provider_from_string, provider_for_role,
};
use openhuman_core::openhuman::inference::provider::{
    list_configured_models, sanitize_api_error, ChatMessage, ChatRequest, Provider, ProviderDelta,
};
use openhuman_core::openhuman::tools::ToolSpec;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<(String, Option<String>, Value)>>>,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: this test binary is run with --test-threads=1 in validation.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: this test binary is run with --test-threads=1 in validation.
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: this test binary is run with --test-threads=1 in validation.
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

#[tokio::test]
async fn compatible_provider_covers_chat_responses_streaming_tools_and_errors() {
    let (base, state) = serve_mock().await;
    let provider = OpenAiCompatibleProvider::new_with_user_agent(
        "custom_openai",
        &format!("{base}/v1"),
        Some("sk-test-secret"),
        CompatibleAuthStyle::Bearer,
        "round15-agent",
    )
    .with_temperature_unsupported_models(vec!["cold-*".to_string()])
    .with_temperature_override(Some(0.7));

    let simple = provider
        .chat_with_system(Some("system"), "hello", "demo-chat", 0.2)
        .await
        .expect("chat_with_system");
    assert_eq!(simple, "chat:demo-chat");

    let history = provider
        .chat_with_history(
            &[ChatMessage::system("rules"), ChatMessage::user("history")],
            "responses-only",
            0.3,
        )
        .await
        .expect("responses fallback");
    assert_eq!(history, "responses fallback text");

    let tools = vec![ToolSpec {
        name: "lookup".to_string(),
        description: "lookup a thing".to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        }),
    }];
    let native = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("use a tool")],
                tools: Some(&tools),
                stream: None,
            },
            "tool-model",
            0.1,
        )
        .await
        .expect("native tools");
    assert_eq!(native.text.as_deref(), Some("tool response"));
    assert_eq!(native.tool_calls.len(), 1);
    assert_eq!(native.tool_calls[0].name, "lookup");
    assert_eq!(native.tool_calls[0].arguments, r#"{"query":"openhuman"}"#);
    let usage = native.usage.expect("usage");
    assert_eq!(usage.input_tokens, 11);
    assert_eq!(usage.output_tokens, 7);
    assert_eq!(usage.cached_input_tokens, 3);
    assert!((usage.charged_amount_usd - 0.0042).abs() < f64::EPSILON);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(16);
    let streamed = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("stream it")],
                tools: Some(&tools),
                stream: Some(&tx),
            },
            "stream-model",
            0.1,
        )
        .await
        .expect("streaming native chat");
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

    let err = provider
        .chat_with_system(None, "boom", "budget-model", 0.2)
        .await
        .expect_err("budget error");
    assert!(err.to_string().contains("budget exhausted"));
    assert_eq!(
        sanitize_api_error("leaked sk-abcdef ghp_secret-token"),
        "leaked [REDACTED] [REDACTED]"
    );

    let cold = provider
        .chat_with_system(None, "no temperature", "cold-no-temp", 0.2)
        .await
        .expect("temperature omitted");
    assert_eq!(cold, "chat:cold-no-temp");

    let seen = state.requests.lock().expect("requests");
    assert!(seen.iter().any(|(path, auth, _)| {
        path == "/v1/chat/completions" && auth.as_deref() == Some("Bearer sk-test-secret")
    }));
    assert!(seen
        .iter()
        .any(|(path, _, body)| path == "/v1/responses" && body["instructions"] == "rules"));
    assert!(seen.iter().any(|(_, _, body)| {
        body.get("temperature").is_none() && body["model"] == "cold-no-temp"
    }));
}

#[tokio::test]
async fn provider_factory_and_model_listing_cover_cloud_local_and_invalid_shapes() {
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    let _workspace_env = EnvVarGuard::set(
        "OPENHUMAN_WORKSPACE",
        config.config_path.parent().expect("config parent"),
    );
    seed_session(&config);
    config.cloud_providers = vec![
        CloudProviderCreds {
            id: "custom-id".to_string(),
            slug: "custom".to_string(),
            label: "Custom".to_string(),
            endpoint: format!("{base}/v1"),
            auth_style: CloudAuthStyle::Bearer,
            legacy_type: None,
            default_model: Some("demo-chat".to_string()),
        },
        CloudProviderCreds {
            id: "openrouter-id".to_string(),
            slug: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            endpoint: format!("{base}/openrouter"),
            auth_style: CloudAuthStyle::Bearer,
            legacy_type: None,
            default_model: None,
        },
        CloudProviderCreds {
            id: "missing-id".to_string(),
            slug: "missing".to_string(),
            label: "Missing".to_string(),
            endpoint: format!("{base}/missing"),
            auth_style: CloudAuthStyle::None,
            legacy_type: None,
            default_model: None,
        },
        CloudProviderCreds {
            id: "html-id".to_string(),
            slug: "html".to_string(),
            label: "HTML".to_string(),
            endpoint: format!("{base}/html"),
            auth_style: CloudAuthStyle::None,
            legacy_type: None,
            default_model: None,
        },
        CloudProviderCreds {
            id: "wrong-data-id".to_string(),
            slug: "wrong-data".to_string(),
            label: "Wrong Data".to_string(),
            endpoint: format!("{base}/wrong-data"),
            auth_style: CloudAuthStyle::None,
            legacy_type: None,
            default_model: None,
        },
        CloudProviderCreds {
            id: "error-payload-id".to_string(),
            slug: "error-payload".to_string(),
            label: "Error Payload".to_string(),
            endpoint: format!("{base}/error-payload"),
            auth_style: CloudAuthStyle::None,
            legacy_type: None,
            default_model: None,
        },
    ];
    config.chat_provider = Some("custom:demo-chat@0.4".to_string());
    config.reasoning_provider = None;
    config.local_ai.base_url = Some(base.clone());

    AuthService::from_config(&config)
        .store_provider_token(
            &auth_key_for_slug("custom"),
            DEFAULT_AUTH_PROFILE_NAME,
            "custom-key",
            HashMap::new(),
            true,
        )
        .expect("store custom key");
    AuthService::from_config(&config)
        .store_provider_token(
            &auth_key_for_slug("openrouter"),
            DEFAULT_AUTH_PROFILE_NAME,
            "openrouter-key",
            HashMap::new(),
            true,
        )
        .expect("store openrouter key");
    config.save().await.expect("save temp config");

    assert_eq!(provider_for_role("chat", &config), "custom:demo-chat@0.4");
    assert_eq!(
        provider_for_role("reasoning", &config),
        "custom:demo-chat@0.4"
    );

    let (_provider, model) =
        create_chat_provider_from_string("chat", "custom:demo-chat@0.4", &config)
            .expect("cloud provider");
    assert_eq!(model, "demo-chat");

    let (_local_provider, local_model) =
        create_chat_provider_from_string("chat", "ollama:gemma3:1b-it-qat@0.1", &config)
            .expect("ollama provider");
    assert_eq!(local_model, "gemma3:1b-it-qat");

    let empty_model = match create_chat_provider_from_string("chat", "ollama:", &config) {
        Ok(_) => panic!("expected empty model error"),
        Err(err) => err,
    };
    assert!(empty_model.to_string().contains("empty model"));

    let listed = list_configured_models("custom")
        .await
        .expect("list models")
        .value;
    assert_eq!(listed["models"][0]["id"], "demo-chat");
    assert_eq!(listed["models"][1]["context_window"], 8192);

    let local_listed = list_configured_models("ollama")
        .await
        .expect("synthetic ollama list")
        .value;
    assert_eq!(local_listed["models"][0]["id"], "demo-chat");

    let unsupported = list_configured_models("missing")
        .await
        .expect("404 unsupported")
        .value;
    assert_eq!(unsupported["unsupported"], true);

    let openrouter = list_configured_models("openrouter")
        .await
        .expect("openrouter key validation and list")
        .value;
    assert_eq!(openrouter["models"][0]["owned_by"], "test-suite");

    for provider_id in ["html", "wrong-data", "error-payload", ""] {
        let err = list_configured_models(provider_id)
            .await
            .expect_err("invalid model listing");
        assert!(
            err.contains("provider")
                || err.contains("provider_id")
                || err.contains("OpenRouter")
                || err.contains("parse JSON")
        );
    }
}

#[tokio::test]
async fn local_service_public_inference_assets_and_shutdown_use_loopback_ollama() {
    let (base, _state) = serve_mock().await;
    let _ollama_env = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.base_url = Some(base);
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.vision_model_id = "llava:mock".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.preload_embedding_model = true;
    config.local_ai.preload_vision_model = false;
    config.local_ai.preload_stt_model = false;
    config.local_ai.preload_tts_voice = false;
    config.local_ai.whisper_in_process = false;

    let service = LocalAiService::new(&config);
    let prompt = service
        .prompt(&config, "Say hi", Some(8), true)
        .await
        .expect("prompt");
    assert_eq!(prompt, "generated final");

    let summarized = service
        .summarize(&config, "one two three", Some(16))
        .await
        .expect("summarize");
    assert_eq!(summarized, "generated final");

    let completion = service
        .inline_complete_interactive(
            &config,
            "OpenHuman is",
            "concise",
            Some("short"),
            &["OpenHuman is useful".to_string()],
            Some(6),
        )
        .await
        .expect("inline");
    assert_eq!(completion, "generated final");

    let assets = service.assets_status(&config).await.expect("assets");
    assert!(assets.ollama_available);
    assert_eq!(assets.chat.state, "ready");
    assert_eq!(assets.embedding.state, "ready");
    assert!(matches!(assets.stt.state.as_str(), "ondemand" | "missing"));
    assert!(matches!(
        assets.tts.state.as_str(),
        "ready" | "ondemand" | "missing"
    ));

    let diagnostics = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diagnostics["ollama_running"], true);
    assert_eq!(diagnostics["expected"]["chat_found"], true);
    assert!(
        diagnostics["installed_models"]
            .as_array()
            .expect("installed_models")
            .len()
            >= 4
    );

    let progress = service.downloads_progress(&config).await.expect("progress");
    assert_eq!(progress.chat.id, "gemma3:1b-it-qat");
    assert_eq!(progress.embedding.id, "bge-m3");

    let disabled_config = Config::default();
    let disabled_err = service
        .prompt(&disabled_config, "disabled", None, false)
        .await
        .expect_err("disabled prompt");
    assert_eq!(disabled_err, "local ai is disabled");

    service.shutdown_owned_ollama(&config).await;
    assert!(!service.has_owned_ollama());
}

async fn collect_deltas(rx: &mut tokio::sync::mpsc::Receiver<ProviderDelta>) -> Vec<ProviderDelta> {
    let mut out = Vec::new();
    while let Some(delta) = rx.recv().await {
        out.push(delta);
    }
    out
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

fn seed_session(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "session-token",
            HashMap::new(),
            true,
        )
        .expect("seed session");
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/v1/models", get(models))
        .route("/missing/models", get(missing_models))
        .route("/html/models", get(html_models))
        .route("/wrong-data/models", get(wrong_data_models))
        .route("/error-payload/models", get(error_payload_models))
        .route("/openrouter/key", get(openrouter_key))
        .route("/openrouter/models", get(models))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/generate", post(ollama_generate))
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
    let model = body["model"].as_str().unwrap_or_default();
    if model == "responses-only" {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response();
    }
    if model == "budget-model" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "budget exhausted for sk-very-secret"}})),
        )
            .into_response();
    }
    if model == "stream-model" && body["stream"] == true {
        return sse_response();
    }
    if model == "tool-model" {
        return Json(json!({
            "choices": [{
                "message": {
                    "content": "tool response",
                    "reasoning_content": "hidden reasoning",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "lookup",
                            "arguments": { "query": "openhuman" }
                        }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 2,
                "prompt_tokens_details": { "cached_tokens": 1 }
            },
            "openhuman": {
                "usage": {
                    "input_tokens": 11,
                    "output_tokens": 7,
                    "cached_input_tokens": 3
                },
                "billing": { "charged_amount_usd": 0.0042 }
            }
        }))
        .into_response();
    }
    Json(json!({
        "choices": [{
            "message": {
                "content": format!("chat:{model}"),
                "function_call": {
                    "name": "legacy_tool",
                    "arguments": "{\"ok\":true}"
                }
            }
        }]
    }))
    .into_response()
}

async fn responses(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/responses", &headers, body);
    Json(json!({
        "output": [{
            "content": [{ "type": "output_text", "text": "nested fallback text" }]
        }],
        "output_text": "responses fallback text"
    }))
}

fn sse_response() -> Response<Body> {
    let chunks = [
        json!({"choices":[{"delta":{"content":"hello "}}]}).to_string(),
        json!({"choices":[{"delta":{"reasoning_content":"thinking"}}]}).to_string(),
        json!({"choices":[{"delta":{"content":"world","tool_calls":[{
            "index":0,
            "id":"call_stream",
            "type":"function",
            "function":{"name":"lookup","arguments":"{\"query\""}
        }]}}]})
        .to_string(),
        json!({"choices":[{"delta":{"tool_calls":[{
            "index":0,
            "function":{"arguments":":\"stream\"}"}
        }]}}]})
        .to_string(),
        json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 6,
                "prompt_tokens_details": { "cached_tokens": 2 }
            }
        })
        .to_string(),
    ];
    let body = chunks
        .into_iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .chain(std::iter::once("data: [DONE]\n\n".to_string()))
        .collect::<String>();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .expect("sse response")
}

async fn models(State(state): State<MockState>, headers: HeaderMap) -> impl IntoResponse {
    remember(&state, "/v1/models", &headers, Value::Null);
    Json(json!({
        "object": "list",
        "data": [
            { "id": "demo-chat", "owned_by": "test-suite" },
            { "id": "demo-coder", "owned_by": "test-suite", "context_window": 8192 },
            { "owned_by": "ignored-without-id" }
        ]
    }))
}

async fn missing_models() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": "models unsupported"})),
    )
}

async fn html_models() -> impl IntoResponse {
    (StatusCode::OK, "<html>login</html>")
}

async fn wrong_data_models() -> impl IntoResponse {
    Json(json!({ "object": "error", "data": { "message": "wrong shape" } }))
}

async fn error_payload_models() -> impl IntoResponse {
    Json(json!({ "error": { "message": "bad sk-secret-key" } }))
}

async fn openrouter_key(headers: HeaderMap) -> impl IntoResponse {
    let auth = header_value(&headers, "authorization").unwrap_or_default();
    if auth == "Bearer openrouter-key" {
        Json(json!({"data": {"label": "ok"}})).into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing key"})),
        )
            .into_response()
    }
}

async fn ollama_tags() -> impl IntoResponse {
    Json(json!({
        "models": [
            { "name": "gemma3:1b-it-qat", "model": "gemma3:1b-it-qat" },
            { "name": "llava:mock", "model": "llava:mock" },
            { "name": "bge-m3", "model": "bge-m3" },
            { "name": "demo-chat", "model": "demo-chat" }
        ]
    }))
}

async fn ollama_show(Json(body): Json<Value>) -> impl IntoResponse {
    let model = body
        .get("model")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if model == "___nonexistent_probe___" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "model not found"})),
        )
            .into_response();
    }
    Json(json!({
        "model_info": {
            "general.context_length": if model == "bge-m3" { 8192 } else { 4096 }
        }
    }))
    .into_response()
}

async fn ollama_generate(Json(_body): Json<Value>) -> impl IntoResponse {
    Json(json!({
        "model": "gemma3:1b-it-qat",
        "response": "generated final",
        "done": true,
        "prompt_eval_count": 8,
        "prompt_eval_duration": 400000000,
        "eval_count": 6,
        "eval_duration": 300000000
    }))
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state.requests.lock().expect("requests").push((
        path.to_string(),
        header_value(headers, "authorization"),
        body,
    ));
}

fn header_value(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}
