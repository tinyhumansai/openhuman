//! Round 22 raw/E2E-style coverage for inference provider/admin branches.
//!
//! All external inference/admin surfaces are mocked with loopback HTTP servers
//! and temp PATH binaries. This suite must not invoke real Ollama, MLX, Python,
//! whisper, piper, local AI binaries, models, or downloads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{stream, StreamExt};
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
    auth_key_for_slug, create_chat_provider_from_string,
};
use openhuman_core::openhuman::inference::provider::reliable::ReliableProvider;
use openhuman_core::openhuman::inference::provider::traits::{
    StreamChunk, StreamError, StreamOptions, StreamResult,
};
use openhuman_core::openhuman::inference::provider::{
    list_configured_models, ChatMessage, ChatRequest, ChatResponse, Provider, ToolCall,
};

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<SeenRequest>>>,
}

#[derive(Debug, Clone)]
struct SeenRequest {
    path: String,
    auth: Option<String>,
    user_agent: Option<String>,
    body: Value,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this integration test is validated with --test-threads=1.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this integration test is validated with --test-threads=1.
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: mutation is serialized by `env_lock()` (see below).
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: mutation is serialized by `env_lock()` (see below).
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

/// Serializes the whole suite's process-global env access.
///
/// Several tests mutate `OPENHUMAN_WORKSPACE` / `OPENHUMAN_OLLAMA_BASE_URL` /
/// `PATH` via [`EnvVarGuard`]. `cargo test` (and `cargo llvm-cov`) run a
/// binary's tests on multiple threads by default, so without this lock those
/// mutations race and a test reads another test's workspace/config — observed
/// as a flaky failure under `cargo llvm-cov` (the coverage job does not pass
/// `--test-threads=1`). Every test takes this guard up front so the suite is
/// effectively serialized regardless of the runner's thread count.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[tokio::test]
async fn compatible_provider_covers_responses_fallback_auth_and_merge_system_edges() {
    let _env = env_lock();
    let (base, state) = serve_mock().await;

    let fallback = OpenAiCompatibleProvider::new(
        "round22-compatible",
        &format!("{base}/fallback/v1"),
        Some("sk-round22"),
        CompatibleAuthStyle::Bearer,
    );
    let text = fallback
        .chat_with_history(
            &[
                ChatMessage::system("policy one"),
                ChatMessage::user("use responses fallback"),
            ],
            "fallback-model",
            0.7,
        )
        .await
        .expect("responses fallback");
    assert_eq!(text, "round22 responses text");

    let no_fallback = OpenAiCompatibleProvider::new_no_responses_fallback(
        "round22-no-fallback",
        &format!("{base}/fallback/v1"),
        None,
        CompatibleAuthStyle::None,
    );
    let err = no_fallback
        .chat_with_history(&[ChatMessage::user("no fallback")], "fallback-model", 0.2)
        .await
        .expect_err("404 without responses fallback");
    assert!(err
        .to_string()
        .contains("check that your endpoint URL is correct"));

    let system_only_err = fallback
        .chat_with_history(
            &[ChatMessage::system("only instructions")],
            "fallback-model",
            0.2,
        )
        .await
        .expect_err("responses fallback requires input");
    assert!(system_only_err
        .to_string()
        .contains("requires at least one non-system message"));

    let merged = OpenAiCompatibleProvider::new_merge_system_into_user(
        "minimax",
        &format!("{base}/merge/v1"),
        Some("x-api-secret"),
        CompatibleAuthStyle::XApiKey,
    );
    let merged_text = merged
        .chat_with_history(
            &[
                ChatMessage::system("system policy"),
                ChatMessage::user("hello"),
            ],
            "merge-model",
            0.1,
        )
        .await
        .expect("merge system into user");
    assert_eq!(merged_text, "merged ok");

    let custom = OpenAiCompatibleProvider::new_with_user_agent(
        "custom-auth",
        &format!("{base}/custom-auth/v1"),
        Some("custom-secret"),
        CompatibleAuthStyle::Custom("x-custom-auth".to_string()),
        "Round22UA/1",
    );
    assert_eq!(
        custom
            .chat_with_system(Some("custom policy"), "custom hello", "custom-model", 0.3)
            .await
            .expect("custom auth"),
        "custom auth ok"
    );

    let seen = state.requests.lock().expect("requests");
    let responses = seen
        .iter()
        .find(|req| req.path == "/fallback/v1/responses")
        .expect("responses request");
    assert_eq!(responses.auth.as_deref(), Some("Bearer sk-round22"));
    assert_eq!(responses.body["instructions"], "policy one");
    assert_eq!(responses.body["input"][0]["role"], "user");

    let merged_body = seen
        .iter()
        .find(|req| req.path == "/merge/v1/chat/completions")
        .expect("merge request")
        .body
        .clone();
    assert_eq!(merged_body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(merged_body["messages"][0]["role"], "user");
    assert!(merged_body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("system policy"));
    assert!(seen
        .iter()
        .any(|req| req.path == "/merge/v1/chat/completions"
            && req.auth.as_deref() == Some("x-api-secret")));
    assert!(seen
        .iter()
        .any(|req| req.path == "/custom-auth/v1/chat/completions"
            && req.auth.as_deref() == Some("custom-secret")
            && req.user_agent.as_deref() == Some("Round22UA/1")));
}

#[tokio::test]
async fn provider_admin_model_listing_covers_openrouter_validation_and_local_synthesis() {
    let _env = env_lock();
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.base_url = Some(base.clone());
    config.cloud_providers = vec![
        provider_entry(
            "openrouter-id",
            "openrouter",
            &format!("{base}/openrouter/api/v1"),
            CloudAuthStyle::Bearer,
            None,
        ),
        provider_entry(
            "object-error-id",
            "object-error",
            &format!("{base}/object-error"),
            CloudAuthStyle::None,
            None,
        ),
    ];
    config.save().await.expect("save config");
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        &auth_key_for_slug("openrouter"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-openrouter",
        HashMap::new(),
        true,
    )
    .expect("store openrouter key");
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);

    let openrouter = list_configured_models("openrouter")
        .await
        .expect("openrouter models")
        .value;
    assert_eq!(openrouter["models"][0]["id"], "or-model");

    let object_error = list_configured_models("object-error")
        .await
        .expect_err("object error payload");
    assert!(object_error.contains("nested provider failure"));

    let synthetic_ollama = list_configured_models("ollama")
        .await
        .expect("synthetic ollama /v1 models")
        .value;
    assert_eq!(synthetic_ollama["models"][0]["id"], "ollama-synth");

    config.cloud_providers = vec![provider_entry(
        "openrouter-id",
        "openrouter",
        &format!("{base}/openrouter-bad/api/v1"),
        CloudAuthStyle::Bearer,
        None,
    )];
    config.save().await.expect("save bad openrouter config");
    auth.store_provider_token(
        &auth_key_for_slug("openrouter"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-openrouter-bad",
        HashMap::new(),
        true,
    )
    .expect("store bad openrouter key");
    let bad_key = list_configured_models("openrouter")
        .await
        .expect_err("openrouter key validation body");
    assert!(bad_key.contains("OpenRouter key validation returned error payload"));
    assert!(!bad_key.contains("sk-openrouter-bad"));

    let seen = state.requests.lock().expect("requests");
    assert!(seen.iter().any(|req| req.path == "/openrouter/api/v1/key"
        && req.auth.as_deref() == Some("Bearer sk-openrouter")));
    assert!(seen
        .iter()
        .any(|req| req.path == "/v1/models" && req.auth.is_none()));
}

#[tokio::test]
async fn factory_covers_legacy_api_key_scoping_and_abstract_model_errors() {
    let _env = env_lock();
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.api_key = Some("sk-legacy-direct".to_string());
    config.inference_url = Some(format!("{base}/legacy/v1"));
    config.cloud_providers = vec![
        provider_entry(
            "legacy-id",
            "legacy",
            &format!("{base}/legacy/v1/"),
            CloudAuthStyle::Bearer,
            Some("legacy-default"),
        ),
        provider_entry(
            "other-id",
            "other",
            &format!("{base}/other/v1"),
            CloudAuthStyle::Bearer,
            Some("other-default"),
        ),
        provider_entry(
            "abstract-id",
            "abstract",
            &format!("{base}/abstract/v1"),
            CloudAuthStyle::Bearer,
            None,
        ),
    ];
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        "session-token",
        HashMap::new(),
        true,
    )
    .expect("store app session");
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());

    let (legacy, legacy_model) =
        create_chat_provider_from_string("chat", "legacy:requested-model", &config)
            .expect("legacy direct provider");
    assert_eq!(legacy_model, "requested-model");
    assert_eq!(
        legacy
            .chat_with_system(None, "hello", &legacy_model, 0.4)
            .await
            .expect("legacy chat"),
        "legacy direct ok"
    );

    let (other, other_model) =
        create_chat_provider_from_string("chat", "other:other-model", &config)
            .expect("other provider");
    let other_err = other
        .chat_with_system(None, "hello", &other_model, 0.4)
        .await
        .expect_err("other provider should not inherit legacy direct key");
    assert!(other_err.to_string().contains("API key not set"));

    let abstract_err =
        match create_chat_provider_from_string("reasoning", "abstract:reasoning-v1", &config) {
            Ok(_) => panic!("expected abstract tier error"),
            Err(err) => err,
        };
    assert!(abstract_err
        .to_string()
        .contains("has no concrete default_model configured"));

    let seen = state.requests.lock().expect("requests");
    assert!(seen
        .iter()
        .any(|req| req.path == "/legacy/v1/chat/completions"
            && req.auth.as_deref() == Some("Bearer sk-legacy-direct")));
    assert!(!seen
        .iter()
        .any(|req| req.path == "/other/v1/chat/completions"));
}

#[tokio::test]
async fn reliable_provider_covers_chat_tools_streaming_and_context_bail_edges() {
    let _env = env_lock();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ReliableProvider::new(
        vec![(
            "primary".to_string(),
            Box::new(Round22Provider {
                calls: Arc::clone(&calls),
                mode: Round22Mode::FailsThenSucceeds,
            }) as Box<dyn Provider>,
        )],
        1,
        50,
    );
    let response = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("retry me")],
                tools: None,
                stream: None,
            },
            "retry-model",
            0.2,
        )
        .await
        .expect("chat retry");
    assert_eq!(response.text.as_deref(), Some("chat recovered"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let tool_provider = ReliableProvider::new(
        vec![(
            "tools".to_string(),
            Box::new(Round22Provider {
                calls: Arc::new(AtomicUsize::new(0)),
                mode: Round22Mode::ToolsOk,
            }) as Box<dyn Provider>,
        )],
        0,
        50,
    );
    let tools = tool_provider
        .chat_with_tools(&[ChatMessage::user("tool")], &[], "tool-model", 0.0)
        .await
        .expect("tools");
    assert!(tools.has_tool_calls());
    assert_eq!(tools.tool_calls[0].name, "round22_tool");

    let context_provider = ReliableProvider::new(
        vec![(
            "context".to_string(),
            Box::new(Round22Provider {
                calls: Arc::new(AtomicUsize::new(0)),
                mode: Round22Mode::ContextExceeded,
            }) as Box<dyn Provider>,
        )],
        2,
        50,
    );
    let context_err = context_provider
        .chat_with_history(&[ChatMessage::user("too long")], "tiny-context", 0.0)
        .await
        .expect_err("context is non-retryable bail");
    assert!(context_err
        .to_string()
        .contains("Request exceeds model context window"));

    let disabled_stream = tool_provider
        .stream_chat_with_system(
            None,
            "disabled",
            "stream-model",
            0.0,
            StreamOptions::new(false),
        )
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        &disabled_stream[0],
        Err(StreamError::Provider(message)) if message == "Streaming disabled"
    ));

    let streaming = ReliableProvider::new(
        vec![
            (
                "bad-stream".to_string(),
                Box::new(Round22Provider {
                    calls: Arc::new(AtomicUsize::new(0)),
                    mode: Round22Mode::StreamNonRetryable,
                }) as Box<dyn Provider>,
            ),
            (
                "good-stream".to_string(),
                Box::new(Round22Provider {
                    calls: Arc::new(AtomicUsize::new(0)),
                    mode: Round22Mode::StreamOk,
                }) as Box<dyn Provider>,
            ),
        ],
        0,
        50,
    );
    let chunks = streaming
        .stream_chat_with_system(
            None,
            "stream",
            "stream-model",
            0.0,
            StreamOptions::new(true),
        )
        .collect::<Vec<_>>()
        .await;
    assert!(chunks
        .iter()
        .any(|chunk| chunk.as_ref().is_ok_and(|c| c.delta == "stream ok")));
    assert!(chunks
        .iter()
        .any(|chunk| chunk.as_ref().is_ok_and(|c| c.is_final)));
}

#[tokio::test]
async fn local_admin_covers_diagnostics_errors_assets_status_and_shutdown_with_fake_bins() {
    let _env = env_lock();
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "gemma4:e4b-it-q8_0".to_string();
    config.local_ai.embedding_model_id = "all-minilm:latest".to_string();
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.preload_embedding_model = true;
    config.local_ai.preload_stt_model = true;
    config.local_ai.preload_tts_voice = true;
    config.local_ai.stt_model_id = "round22-stt".to_string();
    config.local_ai.tts_voice_id = "round22-voice".to_string();

    let scripts = tempdir().expect("scripts");
    let ollama = write_stub_script(&scripts, "ollama", "#!/bin/sh\nprintf 'fake ollama\\n'\n");
    write_stub_script(&scripts, "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(&scripts, "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(&scripts, "mlx_lm.generate", "#!/bin/sh\nexit 42\n");
    write_stub_script(&scripts, "piper", "#!/bin/sh\nexit 42\n");

    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _ollama_bin = EnvVarGuard::set("OLLAMA_BIN", &ollama);
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _whisper_bin = EnvVarGuard::unset("WHISPER_BIN");

    let service = LocalAiService::new(&config);
    let diag = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(diag["ollama_running"], true);
    let issues = diag["issues"].as_array().expect("issues");
    assert!(issues.iter().any(|issue| issue
        .as_str()
        .unwrap()
        .contains("Chat model `gemma4:e4b-it-q8_0`")));
    assert!(issues.iter().any(|issue| issue
        .as_str()
        .unwrap()
        .contains("Embedding model `all-minilm:latest`")));

    let mut tags_500 = config.clone();
    tags_500.local_ai.base_url = Some(format!("{base}/tags-500"));
    let diag_500 = service
        .diagnostics(&tags_500)
        .await
        .expect("500 diagnostics");
    assert_eq!(diag_500["ollama_running"], false);
    assert!(diag_500["issues"][0]
        .as_str()
        .unwrap()
        .contains("not running or not reachable"));

    let assets = service.assets_status(&config).await.expect("assets status");
    assert!(assets.ollama_available);
    assert_eq!(assets.chat.state, "missing");
    assert_eq!(assets.embedding.state, "missing");
    assert_ne!(assets.stt.state, "ready");
    assert_ne!(assets.tts.state, "ready");

    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 30")
        .spawn()
        .expect("spawn fake owned ollama child");
    service.inject_owned_ollama(child);
    assert!(service.has_owned_ollama());
    service.shutdown_owned_ollama(&config).await;
    assert!(!service.has_owned_ollama());
}

#[derive(Clone, Copy)]
enum Round22Mode {
    FailsThenSucceeds,
    ToolsOk,
    ContextExceeded,
    StreamNonRetryable,
    StreamOk,
}

struct Round22Provider {
    calls: Arc<AtomicUsize>,
    mode: Round22Mode,
}

#[async_trait]
impl Provider for Round22Provider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        match self.mode {
            Round22Mode::ContextExceeded => {
                anyhow::bail!("400 context_length_exceeded: maximum context length")
            }
            _ => Ok("system ok".to_string()),
        }
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        match self.mode {
            Round22Mode::ContextExceeded => {
                anyhow::bail!("400 context_length_exceeded: maximum context length")
            }
            _ => Ok("history ok".to_string()),
        }
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        match self.mode {
            Round22Mode::FailsThenSucceeds => {
                let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt == 1 {
                    anyhow::bail!("503 service unavailable Retry-After: 0")
                }
                Ok(ChatResponse {
                    text: Some("chat recovered".to_string()),
                    ..ChatResponse::default()
                })
            }
            Round22Mode::ContextExceeded => {
                anyhow::bail!("400 context_length_exceeded: maximum context length")
            }
            _ => Ok(ChatResponse {
                text: Some("chat ok".to_string()),
                ..ChatResponse::default()
            }),
        }
    }

    async fn chat_with_tools(
        &self,
        _messages: &[ChatMessage],
        _tools: &[Value],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some("tool response".to_string()),
            tool_calls: vec![ToolCall {
                id: "round22-call".to_string(),
                name: "round22_tool".to_string(),
                arguments: "{}".to_string(),
                extra_content: None,
            }],
            usage: None,
            reasoning_content: None,
        })
    }

    fn supports_streaming(&self) -> bool {
        matches!(
            self.mode,
            Round22Mode::StreamNonRetryable | Round22Mode::StreamOk
        )
    }

    fn stream_chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
        _options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        match self.mode {
            Round22Mode::StreamNonRetryable => {
                stream::once(async { Err(StreamError::Provider("invalid api key".to_string())) })
                    .boxed()
            }
            Round22Mode::StreamOk => stream::iter(vec![
                Ok(StreamChunk::delta("stream ok")),
                Ok(StreamChunk::final_chunk()),
            ])
            .boxed(),
            _ => stream::empty().boxed(),
        }
    }
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/fallback/v1/chat/completions", post(always_404))
        .route("/fallback/v1/responses", post(responses_fallback))
        .route("/merge/v1/chat/completions", post(merge_chat))
        .route("/custom-auth/v1/chat/completions", post(custom_auth_chat))
        .route("/openrouter/api/v1/key", get(openrouter_key_ok))
        .route("/openrouter/api/v1/models", get(openrouter_models))
        .route("/openrouter-bad/api/v1/key", get(openrouter_key_bad))
        .route("/object-error/models", get(object_error_models))
        .route("/v1/models", get(synthetic_ollama_models))
        .route("/legacy/v1/chat/completions", post(legacy_chat))
        .route("/other/v1/chat/completions", post(other_chat))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/tags-500/api/tags", get(tags_500))
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

async fn always_404(State(state): State<MockState>, headers: HeaderMap) -> impl IntoResponse {
    remember(
        &state,
        "/fallback/v1/chat/completions",
        &headers,
        Value::Null,
    );
    (StatusCode::NOT_FOUND, "missing chat endpoint").into_response()
}

async fn responses_fallback(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/fallback/v1/responses", &headers, body);
    Json(json!({"output_text": "round22 responses text"})).into_response()
}

async fn merge_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/merge/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"merged ok"}}]})).into_response()
}

async fn custom_auth_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/custom-auth/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"custom auth ok"}}]})).into_response()
}

async fn openrouter_key_ok(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/openrouter/api/v1/key", &headers, Value::Null);
    Json(json!({"data": {"label": "ok"}})).into_response()
}

async fn openrouter_models() -> impl IntoResponse {
    Json(json!({"object":"list","data":[{"id":"or-model","owned_by":"openrouter"}]}))
}

async fn openrouter_key_bad(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/openrouter-bad/api/v1/key", &headers, Value::Null);
    Json(json!({"error": {"message": "bad key sk-openrouter-bad"}})).into_response()
}

async fn object_error_models() -> impl IntoResponse {
    Json(json!({"error": {"message": "nested provider failure"}}))
}

async fn synthetic_ollama_models(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/v1/models", &headers, Value::Null);
    Json(json!({"object":"list","data":[{"id":"ollama-synth","context_length":4096}]}))
}

async fn legacy_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/legacy/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"legacy direct ok"}}]})).into_response()
}

async fn other_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/other/v1/chat/completions", &headers, body);
    Json(json!({"choices":[{"message":{"content":"other no key ok"}}]})).into_response()
}

async fn ollama_tags() -> impl IntoResponse {
    Json(json!({
        "models": [
            {"name": "round22-existing", "model": "round22-existing", "size": 1}
        ]
    }))
}

async fn ollama_show() -> impl IntoResponse {
    Json(json!({"model_info": {"general.context_length": 8192}}))
}

async fn tags_500() -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "tags failed").into_response()
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state.requests.lock().expect("requests").push(SeenRequest {
        path: path.to_string(),
        auth: auth_header(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        body,
    });
}

fn auth_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .or_else(|| headers.get("x-api-key"))
        .or_else(|| headers.get("x-custom-auth"))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn provider_entry(
    id: &str,
    slug: &str,
    endpoint: &str,
    auth_style: CloudAuthStyle,
    default_model: Option<&str>,
) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style,
        legacy_type: None,
        default_model: default_model.map(ToString::to_string),
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

fn write_stub_script(tmp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = tmp.path().join(name);
    std::fs::write(&path, body).expect("write stub");
    make_executable(&path);
    path
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }
}
