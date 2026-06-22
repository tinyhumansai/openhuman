//! Round 16 raw/E2E coverage for inference local-admin branches.
//!
//! This suite uses temp workspaces, temp PATH scripts, and loopback HTTP mocks
//! only. It must not call host Ollama, Piper, Whisper, Python, or MLX binaries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

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
use openhuman_core::openhuman::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};
use openhuman_core::openhuman::inference::local::ops::{
    local_ai_chat, local_ai_download_asset, local_ai_downloads_progress, local_ai_should_react,
    LocalAiChatMessage,
};
use openhuman_core::openhuman::inference::local::LocalAiService;
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle as CompatibleAuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::inference::provider::factory::auth_key_for_slug;
use openhuman_core::openhuman::inference::provider::{
    list_configured_models, ChatMessage, ChatRequest, Provider, ProviderDelta,
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
        // SAFETY: tests that mutate environment variables hold env_lock().
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: tests that mutate environment variables hold env_lock().
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: tests that mutate environment variables hold env_lock().
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: tests that mutate environment variables hold env_lock().
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

/// Process-wide lock serializing tests that mutate global environment
/// variables through [`EnvVarGuard`]. `cargo llvm-cov` runs integration tests
/// multi-threaded (it does not pass `--test-threads=1`), so without this guard
/// concurrent tests clobber each other's env — e.g. one test points
/// `OPENHUMAN_OLLAMA_BASE_URL` at an unreachable port and asserts Ollama is
/// unavailable while another points it at a mock and asserts it is available.
/// Each env-mutating test holds this guard for its whole body; declaring it
/// before any `EnvVarGuard` makes it drop last, after the env is restored.
fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tokio::test]
async fn compatible_provider_covers_retry_headers_responses_and_parse_errors() {
    let (base, state) = serve_mock().await;
    let tools = vec![
        ToolSpec {
            name: "lookup".to_string(),
            description: "first wins".to_string(),
            parameters: json!({"type": "object"}),
        },
        ToolSpec {
            name: "lookup".to_string(),
            description: "duplicate should be dropped".to_string(),
            parameters: json!({"type": "object"}),
        },
    ];

    let provider = OpenAiCompatibleProvider::new_merge_system_into_user(
        "custom_openai",
        &format!("{base}/v1"),
        Some("secret-key"),
        CompatibleAuthStyle::Custom("x-custom-auth".to_string()),
    )
    .with_openhuman_thread_id();

    let merged = provider
        .chat_with_system(Some("system line"), "user line", "merge-model", 0.6)
        .await
        .expect("merged chat");
    assert_eq!(merged, "merged response");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(8);
    let retried = provider
        .chat(
            ChatRequest {
                messages: &[
                    ChatMessage::assistant(
                        json!({
                            "content": "called",
                            "reasoning_content": "reasoned",
                            "tool_calls": [{
                                "id": "call_a",
                                "name": "lookup",
                                "arguments": "{\"q\":\"a\"}"
                            }]
                        })
                        .to_string(),
                    ),
                    ChatMessage::tool(
                        json!({
                            "tool_call_id": "call_a",
                            "content": "tool output"
                        })
                        .to_string(),
                    ),
                    ChatMessage::user("stream with retry"),
                ],
                tools: Some(&tools),
                stream: Some(&tx),
                max_tokens: None,
            },
            "stream-tools-unsupported",
            0.2,
        )
        .await
        .expect("stream retry without tools");
    drop(tx);
    assert_eq!(retried.text.as_deref(), Some("json stream fallback"));
    assert!(rx.recv().await.is_none(), "non-SSE JSON emits no deltas");

    let no_fallback = OpenAiCompatibleProvider::new_no_responses_fallback(
        "glm",
        &format!("{base}/v1"),
        None,
        CompatibleAuthStyle::None,
    );
    let err = no_fallback
        .chat_with_system(None, "missing", "not-found-model", 0.2)
        .await
        .expect_err("404 should be enriched without responses fallback");
    assert!(err.to_string().contains("endpoint URL"));

    let empty_err = provider
        .chat_with_history(&[ChatMessage::user("empty choices")], "empty-choices", 0.2)
        .await
        .expect_err("empty choices");
    assert!(empty_err.to_string().contains("No response"));

    let responses_err = provider
        .chat_with_history(
            &[ChatMessage::system("only system")],
            "responses-empty-input",
            0.2,
        )
        .await
        .expect_err("responses requires input");
    assert!(!responses_err.to_string().is_empty());

    let bearer = OpenAiCompatibleProvider::new(
        "bearer",
        &format!("{base}/v1"),
        Some("bearer-token"),
        CompatibleAuthStyle::Bearer,
    );
    let x_api = OpenAiCompatibleProvider::new(
        "xapi",
        &format!("{base}/v1"),
        Some("x-api-token"),
        CompatibleAuthStyle::XApiKey,
    );
    let anthropic = OpenAiCompatibleProvider::new(
        "anthropic",
        &format!("{base}/v1"),
        Some("anthropic-token"),
        CompatibleAuthStyle::Anthropic,
    );
    assert_eq!(
        bearer
            .chat_with_system(None, "auth", "auth-model", 0.1)
            .await
            .expect("bearer auth"),
        "auth response"
    );
    assert_eq!(
        x_api
            .chat_with_system(None, "auth", "auth-model", 0.1)
            .await
            .expect("x-api auth"),
        "auth response"
    );
    assert_eq!(
        anthropic
            .chat_with_system(None, "auth", "auth-model", 0.1)
            .await
            .expect("anthropic auth"),
        "auth response"
    );

    let seen = state.requests.lock().expect("requests");
    let merge_body = seen
        .iter()
        .find(|(_, _, body)| body["model"] == "merge-model")
        .expect("merge request")
        .2
        .clone();
    assert_eq!(merge_body["messages"][0]["role"], "user");
    assert!(merge_body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("system line\n\nuser line"));
    assert!(seen
        .iter()
        .any(|(_, auth, body)| body["model"] == "merge-model"
            && auth.as_deref() == Some("secret-key")));
    assert!(seen.iter().any(|(_, auth, body)| {
        body["model"] == "auth-model" && auth.as_deref() == Some("Bearer bearer-token")
    }));
    assert!(seen
        .iter()
        .any(|(_, auth, body)| body["model"] == "auth-model"
            && auth.as_deref() == Some("x-api-token")));
    assert!(seen.iter().any(|(_, auth, body)| {
        body["model"] == "auth-model" && auth.as_deref() == Some("anthropic-token")
    }));
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
async fn local_admin_covers_assets_diagnostics_downloads_and_ops_errors() {
    let _env_guard = env_lock();
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "gemma4:e4b-it-q8_0".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.vision_model_id = "missing-vision".to_string();
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.preload_vision_model = true;
    config.local_ai.preload_embedding_model = true;
    config.local_ai.preload_stt_model = false;
    config.local_ai.preload_tts_voice = false;
    config.local_ai.tts_voice_id = "round16-voice".to_string();
    config.local_ai.tts_download_url = Some(format!("{base}/asset/tts"));
    config.local_ai.tts_config_download_url = Some(format!("{base}/asset/tts-config-fails"));
    config.local_ai.stt_download_url = None;

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
    assert_eq!(diagnostics["expected"]["chat_found"], false);
    assert_eq!(diagnostics["expected"]["embedding_found"], true);
    assert_eq!(diagnostics["expected"]["vision_found"], false);
    assert_eq!(diagnostics["ok"], false);
    assert!(diagnostics["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue.as_str().unwrap().contains("gemma4:e4b-it-q8_0")));

    let assets = service.assets_status(&config).await.expect("assets");
    assert!(assets.ollama_available);
    assert_eq!(assets.chat.state, "missing");
    assert_eq!(assets.vision.state, "missing");
    assert_eq!(assets.embedding.state, "ready");
    assert_eq!(assets.stt.state, "missing");
    assert_eq!(assets.tts.state, "ondemand");

    let unknown = service
        .download_asset(&config, " nope ")
        .await
        .expect_err("unknown asset");
    assert!(unknown.contains("Unknown capability"));

    let stt_missing_url = service
        .download_asset(&config, "stt")
        .await
        .expect_err("stt without url");
    assert!(stt_missing_url.contains("no local_ai.stt_download_url"));

    let after_tts = service
        .download_asset(&config, "tts")
        .await
        .expect("tts download succeeds even if sidecar url fails");
    assert_eq!(after_tts.tts.state, "ready");
    let progress = service.downloads_progress(&config).await.expect("progress");
    assert_eq!(progress.tts.state, "ready");
    assert_eq!(progress.warning, Some("Downloading tts asset".to_string()));

    let after_chat = service
        .download_asset(&config, "chat")
        .await
        .expect("ollama pull chat model");
    assert_eq!(after_chat.chat.state, "ready");
    assert!(state
        .ollama_models
        .lock()
        .expect("models")
        .iter()
        .any(|m| m == "gemma4:e4b-it-q8_0"));

    let mut lm_config = config.clone();
    lm_config.local_ai.provider = "lmstudio".to_string();
    let lm_err = service
        .download_asset(&lm_config, "chat")
        .await
        .expect_err("lm studio owns chat downloads");
    assert!(lm_err.contains("LM Studio manages"));

    let mut disabled_config = config.clone();
    disabled_config.local_ai.runtime_enabled = false;
    let disabled_err = service
        .download_asset(&disabled_config, "embedding")
        .await
        .expect_err("disabled");
    assert_eq!(disabled_err, "local ai is disabled");

    let empty_chat = local_ai_chat(&config, vec![], None)
        .await
        .expect_err("empty chat");
    assert_eq!(empty_chat, "messages must not be empty");
    let bad_role = local_ai_chat(
        &config,
        vec![LocalAiChatMessage {
            role: "moderator".to_string(),
            content: "hello".to_string(),
        }],
        None,
    )
    .await
    .expect_err("bad role");
    assert!(bad_role.contains("unsupported message role"));

    let reaction = local_ai_should_react(&config, "", "discord")
        .await
        .expect("empty reaction")
        .value;
    assert!(!reaction.should_react);
    assert!(reaction.emoji.is_none());

    let ops_progress = local_ai_downloads_progress(&config)
        .await
        .expect("ops progress")
        .value;
    assert_eq!(ops_progress.chat.id, "gemma4:e4b-it-q8_0");

    let ops_asset = local_ai_download_asset(&config, "embedding")
        .await
        .expect("ops embedding")
        .value;
    assert_eq!(ops_asset.embedding.state, "ready");
}

#[tokio::test]
async fn provider_model_listing_covers_local_synthesis_and_openrouter_failures() {
    let _env_guard = env_lock();
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.base_url = Some(base.clone());
    config.cloud_providers = vec![
        CloudProviderCreds {
            id: "openrouter-id".to_string(),
            slug: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            endpoint: format!("{base}/openrouter-error"),
            auth_style: CloudAuthStyle::Bearer,
            legacy_type: None,
            default_model: None,
        },
        CloudProviderCreds {
            id: "array-id".to_string(),
            slug: "array-body".to_string(),
            label: "Array Body".to_string(),
            endpoint: format!("{base}/array-body"),
            auth_style: CloudAuthStyle::None,
            legacy_type: None,
            default_model: None,
        },
        CloudProviderCreds {
            id: "status-id".to_string(),
            slug: "status-body".to_string(),
            label: "Status Body".to_string(),
            endpoint: format!("{base}/status-body"),
            auth_style: CloudAuthStyle::None,
            legacy_type: None,
            default_model: None,
        },
    ];
    config.save().await.expect("save config");
    AuthService::from_config(&config)
        .store_provider_token(
            &auth_key_for_slug("openrouter"),
            DEFAULT_AUTH_PROFILE_NAME,
            "sk-openrouter-secret",
            HashMap::new(),
            true,
        )
        .expect("store token");

    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);

    let local = list_configured_models("ollama")
        .await
        .expect("synthetic ollama")
        .value;
    assert_eq!(local["models"][0]["id"], "bge-m3");

    let array_err = list_configured_models("array-body")
        .await
        .expect_err("top-level array");
    assert!(array_err.contains("not a JSON object"));

    let status_err = list_configured_models("status-body")
        .await
        .expect_err("non-success");
    assert!(status_err.contains("provider returned 500"));
    assert!(!status_err.contains("sk-status-secret"));

    let openrouter_err = list_configured_models("openrouter")
        .await
        .expect_err("openrouter key validation error payload");
    assert!(openrouter_err.contains("OpenRouter key validation returned error payload"));
    assert!(!openrouter_err.contains("sk-openrouter-secret"));
}

#[tokio::test]
async fn local_admin_reports_unhealthy_runtime_and_lm_studio_issue_shapes() {
    let _env_guard = env_lock();
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.base_url = Some("http://127.0.0.1:9".to_string());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:9");
    let service = LocalAiService::new(&config);

    let unhealthy = service.diagnostics(&config).await.expect("unhealthy diag");
    assert_eq!(unhealthy["ollama_running"], false);
    assert!(unhealthy["issues"][0]
        .as_str()
        .unwrap()
        .contains("not running or not reachable"));
    let assets = service
        .assets_status(&config)
        .await
        .expect("unhealthy assets");
    assert!(!assets.ollama_available);
    assert_eq!(assets.chat.state, "missing");

    let (base, _state) = serve_mock().await;
    let mut lm_config = config.clone();
    lm_config.local_ai.provider = "lm-studio".to_string();
    lm_config.local_ai.base_url = Some(format!("{base}/lm-empty/v1"));
    lm_config.local_ai.chat_model_id = "loaded-chat".to_string();
    let lm_empty = service
        .diagnostics(&lm_config)
        .await
        .expect("lm studio empty");
    assert_eq!(lm_empty["provider"], "lm_studio");
    assert_eq!(lm_empty["lm_studio_running"], true);
    assert!(lm_empty["issues"][0]
        .as_str()
        .unwrap()
        .contains("no models are loaded"));

    lm_config.local_ai.base_url = Some(format!("{base}/lm-wrong/v1"));
    let lm_wrong = service
        .diagnostics(&lm_config)
        .await
        .expect("lm studio wrong model");
    assert!(lm_wrong["issues"][0]
        .as_str()
        .unwrap()
        .contains("not loaded"));

    lm_config.local_ai.base_url = Some(format!("{base}/lm-error/v1"));
    let lm_error = service
        .diagnostics(&lm_config)
        .await
        .expect("lm studio error payload");
    assert!(lm_error["issues"][0]
        .as_str()
        .unwrap()
        .contains("no models are loaded"));
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    *state.ollama_models.lock().expect("models") =
        vec!["bge-m3".to_string(), "loaded-chat".to_string()];
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/v1/models", get(models))
        .route("/array-body/models", get(array_body_models))
        .route("/status-body/models", get(status_body_models))
        .route("/openrouter-error/key", get(openrouter_error_key))
        .route("/openrouter-error/models", get(models))
        .route("/lm-empty/v1/models", get(empty_lm_models))
        .route("/lm-wrong/v1/models", get(wrong_lm_models))
        .route("/lm-error/v1/models", get(error_payload_models))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/pull", post(ollama_pull))
        .route("/api/generate", post(ollama_generate))
        .route("/api/chat", post(ollama_chat))
        .route("/asset/tts", get(asset_tts))
        .route("/asset/tts-config-fails", get(asset_tts_config_fails))
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
    match model {
        "merge-model" => Json(json!({
            "choices": [{ "message": { "content": "merged response" } }]
        }))
        .into_response(),
        "stream-tools-unsupported" if body.get("tools").is_some() => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "model does not support tools"}})),
        )
            .into_response(),
        "stream-tools-unsupported" => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "choices": [{ "message": { "content": "json stream fallback" } }]
                })
                .to_string(),
            ))
            .expect("json response")
            .into_response(),
        "not-found-model" | "responses-empty-input" => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"message": "not found sk-should-redact"}})),
        )
            .into_response(),
        "empty-choices" => Json(json!({ "choices": [] })).into_response(),
        "auth-model" => Json(json!({
            "choices": [{ "message": { "content": "auth response" } }]
        }))
        .into_response(),
        _ => Json(json!({
            "choices": [{ "message": { "content": "default response" } }]
        }))
        .into_response(),
    }
}

async fn responses(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/responses", &headers, body.clone());
    if body["input"]
        .as_str()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        return Json(json!({ "output": [] })).into_response();
    }
    Json(json!({
        "output": [{
            "content": [{ "type": "output_text", "text": "responses fallback" }]
        }]
    }))
    .into_response()
}

async fn models(State(state): State<MockState>) -> impl IntoResponse {
    let models = state
        .ollama_models
        .lock()
        .expect("models")
        .iter()
        .map(|id| json!({ "id": id, "owned_by": "round16", "context_window": 8192 }))
        .collect::<Vec<_>>();
    Json(json!({ "object": "list", "data": models }))
}

async fn array_body_models() -> impl IntoResponse {
    Json(json!([{ "id": "bad" }]))
}

async fn status_body_models() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "status failed sk-status-secret"})),
    )
}

async fn openrouter_error_key() -> impl IntoResponse {
    Json(json!({"error": {"message": "bad key sk-openrouter-secret"}}))
}

async fn empty_lm_models() -> impl IntoResponse {
    Json(json!({ "object": "list", "data": [] }))
}

async fn wrong_lm_models() -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": [{ "id": "some-other-chat", "owned_by": "round16" }]
    }))
}

async fn error_payload_models() -> impl IntoResponse {
    Json(json!({ "error": { "message": "LM Studio endpoint error" } }))
}

async fn ollama_tags(State(state): State<MockState>) -> impl IntoResponse {
    let models = state
        .ollama_models
        .lock()
        .expect("models")
        .iter()
        .map(|name| json!({ "name": name, "model": name }))
        .collect::<Vec<_>>();
    Json(json!({ "models": models }))
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
    let context = match model {
        "gemma4:e4b-it-q8_0" => 1024,
        "bge-m3" => 8192,
        _ => 4096,
    };
    Json(json!({
        "model_info": {
            "general.context_length": context,
            "llama.context_length": context
        }
    }))
    .into_response()
}

async fn ollama_pull(State(state): State<MockState>, Json(body): Json<Value>) -> impl IntoResponse {
    let name = body["name"]
        .as_str()
        .unwrap_or("gemma4:e4b-it-q8_0")
        .to_string();
    state.ollama_models.lock().expect("models").push(name);
    let body = [
        json!({"status":"pulling manifest"}).to_string(),
        json!({"status":"downloading","digest":"sha256:a","total":100,"completed":40}).to_string(),
        json!({"status":"downloading","digest":"sha256:a","total":100,"completed":100}).to_string(),
        json!({"status":"success"}).to_string(),
    ]
    .join("\n")
        + "\n";
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from(body))
        .expect("pull response")
}

async fn ollama_generate() -> impl IntoResponse {
    Json(json!({
        "response": "generated",
        "done": true,
        "prompt_eval_count": 1,
        "prompt_eval_duration": 1000000,
        "eval_count": 1,
        "eval_duration": 1000000
    }))
}

async fn ollama_chat() -> impl IntoResponse {
    Json(json!({
        "message": { "role": "assistant", "content": "chat generated" },
        "done": true
    }))
}

async fn asset_tts() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, "13")
        .body(Body::from("voice-bytes!!"))
        .expect("asset")
}

async fn asset_tts_config_fails() -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "sidecar failed")
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
