//! Round 23 raw/E2E coverage for inference voice/http/local-service gaps.
//!
//! This suite uses temp workspaces, fake binaries, and loopback HTTP/WS servers
//! only. It must not call host Ollama, MLX, Python, Whisper, Piper, models, or
//! download endpoints.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use openhuman_core::core::types::AppState;
use openhuman_core::openhuman::config::schema::cloud_providers::{
    AuthStyle as CloudAuthStyle, CloudProviderCreds,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::inference::http;
use openhuman_core::openhuman::inference::local::{
    local_ai_assets_status, local_ai_downloads_progress, LocalAiService,
};
use openhuman_core::openhuman::inference::voice::streaming::handle_dictation_ws;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<(String, Value)>>>,
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
async fn http_models_and_chat_use_mocked_ollama_without_real_runtime() {
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.default_model = Some("reasoning-v1@0.9".to_string());
    config.chat_provider = Some("ollama:route-chat@0.3".to_string());
    config.reasoning_provider = Some("round23:cloud-chat@0.4".to_string());
    config.local_ai.provider = "ollama".to_string();
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "configured-chat".to_string();
    config.cloud_providers = vec![CloudProviderCreds {
        id: "round23-id".to_string(),
        slug: "round23".to_string(),
        label: "Round 23".to_string(),
        endpoint: format!("{base}/cloud"),
        auth_style: CloudAuthStyle::None,
        legacy_type: None,
        default_model: Some("cloud-default@0.5".to_string()),
    }];
    config.save().await.expect("save config");

    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    store_app_session(&config);

    let app = http::router().with_state(AppState {
        core_version: "round23-test".to_string(),
    });
    let url = serve_app(app).await;
    let client = reqwest::Client::new();

    let models: Value = client
        .get(format!("{url}/models"))
        .send()
        .await
        .expect("models response")
        .json()
        .await
        .expect("models json");
    let ids = models["data"]
        .as_array()
        .expect("models array")
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"openhuman".to_string()));
    assert!(ids.contains(&"reasoning-v1".to_string()));
    assert!(ids.contains(&"ollama:configured-chat".to_string()));
    assert!(ids.contains(&"ollama:route-chat".to_string()));
    assert!(ids.contains(&"round23:cloud-chat".to_string()));
    assert!(!ids.iter().any(|id| id.contains('@')));

    let chat: Value = client
        .post(format!("{url}/chat/completions"))
        .json(&json!({
            "model": "bare-chat",
            "messages": [{ "role": "user", "content": "hello http" }],
            "temperature": 0.2
        }))
        .send()
        .await
        .expect("chat response")
        .json()
        .await
        .expect("chat json");
    assert_eq!(
        chat["choices"][0]["message"]["content"],
        "round23 chat bare-chat"
    );
    assert_eq!(chat["model"], "bare-chat");

    let stream_text = client
        .post(format!("{url}/chat/completions"))
        .json(&json!({
            "model": "ollama:stream-chat",
            "stream": true,
            "messages": [{ "role": "user", "content": "stream please" }]
        }))
        .send()
        .await
        .expect("stream response")
        .text()
        .await
        .expect("stream text");
    assert!(
        stream_text.contains("round23 stream"),
        "stream_text={stream_text}"
    );
    assert!(stream_text.contains("[DONE]"));

    let bad: Value = client
        .post(format!("{url}/chat/completions"))
        .json(&json!({ "model": "ollama:", "messages": [] }))
        .send()
        .await
        .expect("bad response")
        .json()
        .await
        .expect("bad json");
    assert!(bad["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("empty model"));

    let seen = state.requests.lock().expect("requests").clone();
    assert!(seen
        .iter()
        .any(|(path, body)| path == "/v1/chat/completions" && body["model"] == "bare-chat"));
}

#[tokio::test]
async fn dictation_ws_empty_stop_and_audio_cap_do_not_load_whisper() {
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.dictation.streaming = false;
    config.dictation.llm_refinement = false;

    let ws_url = serve_dictation_ws(config).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect empty dictation ws");
    ws.send(WsMessage::Text(r#"{"type":"stop"}"#.to_string()))
        .await
        .expect("send stop");
    let final_msg = ws.next().await.expect("final frame").expect("final ok");
    let final_json: Value =
        serde_json::from_str(final_msg.to_text().expect("text frame")).expect("final json");
    assert_eq!(final_json["type"], "final");
    assert_eq!(final_json["text"], "");
    assert_eq!(final_json["raw_text"], "");

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect capped dictation ws");
    ws.send(WsMessage::Binary(vec![0u8; 9_600_002]))
        .await
        .expect("send oversized pcm");
    let error_msg = ws.next().await.expect("error frame").expect("error ok");
    let error_json: Value =
        serde_json::from_str(error_msg.to_text().expect("text frame")).expect("error json");
    assert_eq!(error_json["type"], "error");
    assert!(error_json["message"]
        .as_str()
        .unwrap_or_default()
        .contains("Recording limit reached"));
}

#[tokio::test]
async fn local_service_assets_and_whisper_fallback_use_fake_files_and_binaries() {
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let scripts = tempdir().expect("scripts");
    let whisper = write_stub_script(
        scripts.path(),
        "whisper-cli",
        "#!/bin/sh\nprintf 'fallback transcript from fake whisper\\n'\n",
    );
    write_stub_script(scripts.path(), "ollama", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "mlx_lm.generate", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "piper", "#!/bin/sh\nexit 42\n");

    let fake_model = tmp.path().join("fake-ggml.bin");
    std::fs::write(&fake_model, b"not a real whisper model").expect("fake model");
    let audio = tmp.path().join("audio.wav");
    std::fs::write(&audio, minimal_wav_16k_mono()).expect("audio wav");

    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.provider = "ollama".to_string();
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.vision_model_id = "vision-ready".to_string();
    config.local_ai.stt_model_id = fake_model.display().to_string();
    config.local_ai.tts_voice_id = "round23-voice".to_string();
    config.local_ai.tts_download_url = Some(format!("{base}/asset/tts"));
    config.local_ai.whisper_in_process = true;
    config.save().await.expect("save config");

    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _whisper_bin = EnvVarGuard::set("WHISPER_BIN", &whisper);
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _ollama_bin = EnvVarGuard::unset("OLLAMA_BIN");

    let service = LocalAiService::new(&config);
    let assets = service.assets_status(&config).await.expect("assets");
    assert!(assets.ollama_available);
    assert_eq!(assets.chat.state, "ready");
    assert_eq!(assets.embedding.state, "ready");
    assert_eq!(assets.vision.state, "ondemand");
    assert_eq!(assets.stt.state, "ready");
    assert_eq!(assets.tts.state, "ondemand");

    let progress = service.downloads_progress(&config).await.expect("progress");
    assert_eq!(progress.stt.state, "ready");
    assert_eq!(progress.tts.state, "ondemand");

    let transcript = service
        .transcribe_with_prompt(
            &config,
            audio.to_string_lossy().as_ref(),
            Some("round23 vocabulary"),
        )
        .await
        .expect("fake whisper fallback transcript");
    assert_eq!(transcript.text, "fallback transcript from fake whisper");
    assert_eq!(transcript.model_id, fake_model.display().to_string());

    assert_eq!(
        local_ai_assets_status(&config)
            .await
            .expect("ops assets")
            .value
            .stt
            .state,
        "ready"
    );
    assert_eq!(
        local_ai_downloads_progress(&config)
            .await
            .expect("ops progress")
            .value
            .tts
            .state,
        "ondemand"
    );
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(ollama_chat_completions))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/asset/tts", get(asset_tts))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    (format!("http://{addr}"), state)
}

async fn serve_app(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind app");
    let addr = listener.local_addr().expect("app addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });
    format!("http://{addr}")
}

async fn serve_dictation_ws(config: Config) -> String {
    let config = Arc::new(config);
    let app = Router::new().route(
        "/ws/dictation",
        get({
            let config = config.clone();
            move |ws: WebSocketUpgrade| {
                let config = config.clone();
                async move { ws.on_upgrade(move |socket| handle_dictation_ws(socket, config)) }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ws");
    let addr = listener.local_addr().expect("ws addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve ws");
    });
    format!("ws://{addr}/ws/dictation")
}

async fn ollama_chat_completions(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response<Body> {
    remember(&state, "/v1/chat/completions", body.clone());
    assert!(
        headers.get(header::AUTHORIZATION).is_none(),
        "ollama-compatible local requests should be authless"
    );
    let model = body["model"].as_str().unwrap_or_default();
    if body["stream"].as_bool().unwrap_or(false) {
        return sse_response([
            json!({"choices":[{"delta":{"content":"round23 stream"}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        ]);
    }
    Json(json!({
        "id": "mock-chat",
        "object": "chat.completion",
        "choices": [{ "message": { "role": "assistant", "content": format!("round23 chat {model}") } }]
    }))
    .into_response()
}

async fn ollama_tags() -> impl IntoResponse {
    Json(json!({
        "models": [
            { "name": "configured-chat", "model": "configured-chat" },
            { "name": "gemma3:1b-it-qat", "model": "gemma3:1b-it-qat" },
            { "name": "bge-m3", "model": "bge-m3" },
            { "name": "vision-ready", "model": "vision-ready" }
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
            "general.context_length": 4096,
            "llama.context_length": 4096
        }
    }))
    .into_response()
}

async fn asset_tts() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, "12")
        .body(Body::from("voice-bytes!"))
        .expect("tts response")
}

fn sse_response<const N: usize>(events: [Value; N]) -> Response<Body> {
    let mut body = events
        .into_iter()
        .map(|event| format!("data: {}\n\n", event))
        .collect::<String>();
    body.push_str("data: [DONE]\n\n");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .expect("sse response")
}

fn remember(state: &MockState, path: &str, body: Value) {
    state
        .requests
        .lock()
        .expect("requests")
        .push((path.to_string(), body));
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

fn store_app_session(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round23-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session");
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

fn minimal_wav_16k_mono() -> Vec<u8> {
    let pcm: [i16; 4] = [0, 100, -100, 0];
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16000u32.to_le_bytes());
    out.extend_from_slice(&32000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}
