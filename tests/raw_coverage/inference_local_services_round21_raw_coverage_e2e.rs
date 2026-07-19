//! Round 21 raw/E2E coverage for local inference service paths.
//!
//! This suite uses temp workspaces, temp PATH scripts, and loopback HTTP mocks only.
//! It must not call host Ollama, MLX, Python, Whisper, Piper, or model binaries.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use openhuman_core::core::all::RegisteredController;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::local::ops::{
    local_ai_assets_status, local_ai_chat, local_ai_download_asset, local_ai_downloads_progress,
    local_ai_prompt, local_ai_should_react, local_ai_transcribe, local_ai_transcribe_bytes,
    LocalAiChatMessage,
};
use openhuman_core::openhuman::inference::local::{
    all_local_inference_registered_controllers, LocalAiService,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    ollama_models: Arc<Mutex<Vec<String>>>,
    whisper_mode: Arc<Mutex<WhisperMode>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum WhisperMode {
    #[default]
    Valid,
    TooSmall,
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

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tokio::test]
async fn local_services_cover_mocked_inference_assets_speech_and_whisper_install() {
    let _lock = env_lock();
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let scripts = tempdir().expect("scripts");
    write_stub_script(
        scripts.path(),
        "whisper-cli",
        "#!/bin/sh\nprintf 'mock whisper transcript\\n'\n",
    );
    write_stub_script(
        scripts.path(),
        "piper",
        "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--output_file\" ]; then shift; out=\"$1\"; fi\n  shift || true\ndone\ncat >/dev/null\nprintf 'RIFFmock' > \"$out\"\n",
    );
    write_stub_script(scripts.path(), "ollama", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "mlx_lm.generate", "#!/bin/sh\nexit 42\n");

    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.provider = "ollama".to_string();
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.vision_model_id = String::new();
    config.local_ai.preload_embedding_model = false;
    config.local_ai.preload_vision_model = false;
    config.local_ai.preload_stt_model = false;
    config.local_ai.preload_tts_voice = false;
    config.local_ai.stt_model_id = "round21-stt.bin".to_string();
    config.local_ai.stt_download_url = Some(format!("{base}/asset/stt"));
    config.local_ai.whisper_in_process = false;
    config.local_ai.tts_voice_id = "round21-voice".to_string();
    config.local_ai.tts_download_url = Some(format!("{base}/asset/tts"));
    config.local_ai.tts_config_download_url = Some(format!("{base}/asset/tts-json"));
    config.save().await.expect("save config");

    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _whisper_models = EnvVarGuard::set("OPENHUMAN_WHISPER_MODELS_BASE_URL", &base);
    let _ollama_bin = EnvVarGuard::unset("OLLAMA_BIN");
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _whisper_bin = EnvVarGuard::unset("WHISPER_BIN");

    let service = LocalAiService::new(&config);

    let initial_assets = service.assets_status(&config).await.expect("assets");
    assert!(initial_assets.ollama_available);
    assert_eq!(initial_assets.chat.state, "ready");
    assert_eq!(initial_assets.vision.state, "disabled");
    assert_eq!(initial_assets.embedding.state, "ready");
    assert_eq!(initial_assets.stt.state, "ondemand");
    assert_eq!(initial_assets.tts.state, "ondemand");

    assert_eq!(
        service
            .prompt(&config, " say hello ", Some(12), true)
            .await
            .expect("prompt"),
        "generated: say hello"
    );
    assert_eq!(
        service
            .summarize(&config, "decision: ship tests", Some(32))
            .await
            .expect("summary"),
        "generated: Summarize this text in concise bullet points. Preserve decisions and commitments.\n\ndecision: ship tests"
    );
    assert_eq!(
        service
            .inline_complete(
                &config,
                "The patch",
                "concise",
                Some("technical"),
                &["The patch adds tests".to_string()],
                Some(8),
            )
            .await
            .expect("inline"),
        "adds tests"
    );

    let after_stt = service
        .download_asset(&config, "stt")
        .await
        .expect("download stt");
    assert_eq!(after_stt.stt.state, "ready");
    let progress = service.downloads_progress(&config).await.expect("progress");
    assert_eq!(progress.stt.state, "ready");
    assert_eq!(progress.warning, Some("Downloading stt asset".to_string()));
    let after_tts = service
        .download_asset(&config, "tts")
        .await
        .expect("download tts");
    assert_eq!(after_tts.tts.state, "ready");

    let audio = tmp.path().join("audio.webm");
    std::fs::write(&audio, b"not real audio").expect("audio");
    let transcribed = service
        .transcribe(&config, audio.to_string_lossy().as_ref())
        .await
        .expect("transcribe via mocked whisper-cli");
    assert_eq!(transcribed.text, "mock whisper transcript");
    assert_eq!(transcribed.model_id, "round21-stt.bin");

    let tts_output = tmp.path().join("out").join("speech.wav");
    let tts = service
        .tts(
            &config,
            "hello from piper",
            Some(tts_output.to_string_lossy().as_ref()),
        )
        .await
        .expect("tts");
    assert_eq!(tts.voice_id, "round21-voice");
    assert!(tts_output.is_file());

    let prompt_outcome = local_ai_prompt(&config, "ops prompt", Some(7), Some(true))
        .await
        .expect("ops prompt")
        .value;
    assert_eq!(prompt_outcome, "generated: ops prompt");
    let chat_outcome = local_ai_chat(
        &config,
        vec![
            LocalAiChatMessage {
                role: "system".to_string(),
                content: "stay short".to_string(),
            },
            LocalAiChatMessage {
                role: "USER".to_string(),
                content: "chat please".to_string(),
            },
        ],
        Some(20),
    )
    .await
    .expect("ops chat")
    .value;
    assert_eq!(chat_outcome, "chat generated");
    let rejected = local_ai_chat(
        &config,
        vec![LocalAiChatMessage {
            role: "critic".to_string(),
            content: "bad role".to_string(),
        }],
        None,
    )
    .await
    .expect_err("bad chat role");
    assert!(rejected.contains("unsupported message role"));

    let reaction = local_ai_should_react(&config, "great work", "discord")
        .await
        .expect("reaction")
        .value;
    assert!(reaction.should_react);
    assert_eq!(reaction.emoji.as_deref(), Some("⭐"));

    assert_eq!(
        local_ai_transcribe(&config, audio.to_string_lossy().as_ref())
            .await
            .expect("ops transcribe")
            .value
            .text,
        "mock whisper transcript"
    );
    assert_eq!(
        local_ai_transcribe_bytes(&config, b"audio bytes", Some(".WEBM".to_string()))
            .await
            .expect("ops transcribe bytes")
            .value
            .text,
        "mock whisper transcript"
    );
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
            .chat
            .id,
        "gemma3:1b-it-qat"
    );
    assert_eq!(
        local_ai_download_asset(&config, "embedding")
            .await
            .expect("ops asset")
            .value
            .embedding
            .state,
        "ready"
    );

    let controllers = all_local_inference_registered_controllers();
    let install = controller(&controllers, "install_whisper");
    let status = controller(&controllers, "whisper_install_status");

    set_whisper_mode(&state, WhisperMode::Valid);
    let queued = call(install, json!({"model_size": "tiny", "force": true}))
        .await
        .expect("queue whisper install");
    assert_eq!(queued["state"], "installing");
    let installed = wait_for_whisper(status, |value| value["state"] == "installed").await;
    assert_eq!(installed["progress"], 100);
    assert_eq!(installed["stage"], "install complete");

    let skipped = call(install, json!({"model_size": "tiny", "force": false}))
        .await
        .expect("queue whisper skip");
    assert!(matches!(
        skipped["state"].as_str(),
        Some("installed") | Some("installing")
    ));
    let skipped = wait_for_whisper(status, |value| {
        value["state"] == "installed" && value["stage"] == "already installed"
    })
    .await;
    assert_eq!(skipped["progress"], 100);

    set_whisper_mode(&state, WhisperMode::TooSmall);
    call(install, json!({"model_size": "smallfail", "force": true}))
        .await
        .expect("queue whisper failure");
    let failed = wait_for_whisper(status, |value| value["state"] == "error").await;
    assert!(failed["error_detail"]
        .as_str()
        .unwrap_or_default()
        .contains("downloaded payload too small"));

    let seen = state.requests.lock().expect("requests").clone();
    assert!(seen.iter().any(|(path, _)| path == "/api/generate"));
    assert!(seen.iter().any(|(path, _)| path == "/api/chat"));
    assert!(seen.iter().any(|(path, _)| path.ends_with("ggml-tiny.bin")));
    assert!(seen
        .iter()
        .any(|(path, _)| path.ends_with("ggml-smallfail.bin")));
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    *state.ollama_models.lock().expect("models") = vec![
        "gemma3:1b-it-qat".to_string(),
        "bge-m3".to_string(),
        "round21-vision".to_string(),
    ];
    let app = Router::new()
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/pull", post(ollama_pull))
        .route("/api/generate", post(ollama_generate))
        .route("/api/chat", post(ollama_chat))
        .route("/asset/stt", get(asset_stt))
        .route("/asset/tts", get(asset_tts))
        .route("/asset/tts-json", get(asset_tts_json))
        .route("/{*path}", get(download_whisper))
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
    Json(json!({
        "model_info": {
            "general.context_length": 8192,
            "llama.context_length": 8192
        }
    }))
    .into_response()
}

async fn ollama_pull(State(state): State<MockState>, Json(body): Json<Value>) -> impl IntoResponse {
    let name = body["name"].as_str().unwrap_or_default().to_string();
    if !name.is_empty() {
        state.ollama_models.lock().expect("models").push(name);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from(
            json!({"status":"success","total":10,"completed":10}).to_string() + "\n",
        ))
        .expect("pull response")
}

async fn ollama_generate(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/api/generate", &headers, body.clone());
    let prompt = body["prompt"].as_str().unwrap_or_default();
    let system = body["system"].as_str().unwrap_or_default();
    let response = if prompt.contains("single emoji character") {
        "⭐".to_string()
    } else if system.contains("inline text completion") {
        "adds tests".to_string()
    } else {
        format!("generated: {}", prompt.trim())
    };
    Json(json!({
        "response": response,
        "done": true,
        "prompt_eval_count": 2,
        "prompt_eval_duration": 1_000_000,
        "eval_count": 4,
        "eval_duration": 2_000_000
    }))
}

async fn ollama_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/api/chat", &headers, body);
    Json(json!({
        "message": { "role": "assistant", "content": "chat generated" },
        "done": true,
        "prompt_eval_count": 1,
        "prompt_eval_duration": 1_000_000,
        "eval_count": 1,
        "eval_duration": 1_000_000
    }))
}

async fn asset_stt() -> impl IntoResponse {
    bytes_response(vec![b's'; 4096])
}

async fn asset_tts() -> impl IntoResponse {
    bytes_response(vec![b't'; 4096])
}

async fn asset_tts_json() -> impl IntoResponse {
    bytes_response(br#"{"audio":{"sample_rate":22050}}"#.to_vec())
}

async fn download_whisper(
    State(state): State<MockState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    remember_path(&state, &format!("/{path}"));
    let mode = *state.whisper_mode.lock().expect("whisper mode");
    match mode {
        WhisperMode::Valid => bytes_response(vec![b'w'; 31 * 1024 * 1024]),
        WhisperMode::TooSmall => bytes_response(vec![b'x'; 1024]),
    }
}

fn bytes_response(bytes: Vec<u8>) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(bytes))
        .expect("bytes response")
}

fn remember(state: &MockState, path: &str, _headers: &HeaderMap, body: Value) {
    state
        .requests
        .lock()
        .expect("requests")
        .push((path.to_string(), body));
}

fn remember_path(state: &MockState, path: &str) {
    state
        .requests
        .lock()
        .expect("requests")
        .push((path.to_string(), Value::Null));
}

fn set_whisper_mode(state: &MockState, mode: WhisperMode) {
    *state.whisper_mode.lock().expect("whisper mode") = mode;
}

async fn wait_for_whisper(status: &RegisteredController, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = Value::Null;
    while Instant::now() < deadline {
        last = call(status, json!({})).await.expect("status");
        if done(&last) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for whisper status, last={last}");
}

fn controller<'a>(
    controllers: &'a [RegisteredController],
    function: &str,
) -> &'a RegisteredController {
    controllers
        .iter()
        .find(|controller| controller.schema.function == function)
        .unwrap_or_else(|| panic!("controller {function} registered"))
}

async fn call(controller: &RegisteredController, params: Value) -> Result<Value, String> {
    let params = params.as_object().cloned().unwrap_or_default();
    (controller.handler)(params).await
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
