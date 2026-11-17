//! Durable native-channel history across actual CLI process restarts.
//!
//! The first process receives a Telegram update and calls a loopback model.
//! After that process exits, a second `openhuman-core serve` process receives a
//! new update for the same Telegram conversation. The second model request
//! must contain the first user's message from the session transcript.
//!
//! Both Telegram and inference are loopback mocks. This target loads a real
//! TinyChannels (and, for voice, TinyVoice) module so it exercises the native
//! Telegram listener through the production TinyBus artifact path: the
//! registry-pinned release the test runner staged, or a local build (see
//! [`TestModules`]).

use std::collections::VecDeque;
use std::fs::File;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio::time::sleep;

const BOT_TOKEN: &str = "session-restart-token";
const MOCK_API_KEY: &str = "loopback-session-restart-key";
const RPC_TOKEN: &str = "native-session-restart-rpc-token";
const FIRST_TEXT: &str = "remember this after the core restarts";
const OTHER_CHAT_TEXT: &str = "remember a separate chat history";
const AFTER_RESET_TEXT: &str = "this starts a fresh session after new";
const RESET_AFTER_RESTART_TEXT: &str = "continue the reset conversation after restart";
const OTHER_AFTER_RESTART_TEXT: &str = "continue the separate chat after restart";
const NATIVE_FILE_BYTES: &[u8] = b"native file bytes for typed admission";

fn native_image_png() -> &'static [u8] {
    static PNG: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PNG.get_or_init(|| {
        base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==")
            .expect("valid 1x1 PNG fixture")
    })
}

#[derive(Default)]
struct MockState {
    updates: Mutex<VecDeque<Value>>,
    update_offsets: Mutex<Vec<i64>>,
    served_update_ids: Mutex<Vec<i64>>,
    file_requests: Mutex<Vec<String>>,
    file_downloads: Mutex<Vec<String>>,
    completions: Mutex<Vec<Value>>,
    sent_messages: Mutex<Vec<Value>>,
    transcription_uploads: Mutex<Vec<Vec<u8>>>,
    voice_file_requests: Mutex<Vec<String>>,
    voice_download_requests: Mutex<usize>,
    speech_requests: Mutex<Vec<String>>,
    sent_voice_uploads: Mutex<Vec<Vec<u8>>>,
}

struct MockServers {
    telegram: String,
    inference: String,
    _telegram_task: JoinHandle<()>,
    _inference_task: JoinHandle<()>,
    state: Arc<MockState>,
}

async fn start_mock_servers() -> MockServers {
    let state = Arc::new(MockState::default());
    let telegram_app = Router::new()
        .route("/botsession-restart-token/getUpdates", post(get_updates))
        .route("/botsession-restart-token/getFile", post(get_file))
        .route("/file/botsession-restart-token/{*path}", get(download_file))
        .route("/botsession-restart-token/sendMessage", post(send_message))
        .route(
            "/botsession-restart-token/editMessageText",
            post(send_message),
        )
        .route("/botsession-restart-token/sendChatAction", post(ok))
        .route("/botsession-restart-token/deleteWebhook", post(ok))
        .route("/botsession-restart-token/getMe", get(get_me))
        .route(
            "/file/botsession-restart-token/voice/input.oga",
            get(download_voice_file),
        )
        .route("/botsession-restart-token/sendVoice", post(send_voice))
        .with_state(Arc::clone(&state));
    let inference_app = Router::new()
        .route("/v1/chat/completions", post(chat_completion))
        .route("/v1/models", get(models))
        .route("/auth/me", get(auth_me))
        .route("/settings", get(auth_me))
        .route("/openai/v1/audio/transcriptions", post(transcribe_voice))
        .route("/openai/v1/audio/speech", post(synthesize_voice))
        .with_state(Arc::clone(&state));
    let (telegram, telegram_task) = serve(telegram_app).await;
    let (inference, inference_task) = serve(inference_app).await;
    MockServers {
        telegram,
        inference,
        _telegram_task: telegram_task,
        _inference_task: inference_task,
        state,
    }
}

async fn serve(app: Router) -> (String, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback mock");
    let address = listener.local_addr().expect("mock address");
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve loopback mock");
    });
    (format!("http://{address}"), task)
}

async fn get_updates(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Json<Value> {
    sleep(Duration::from_millis(25)).await;
    let offset = body
        .get("offset")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    state
        .update_offsets
        .lock()
        .expect("Telegram offsets")
        .push(offset);
    let mut updates = state.updates.lock().expect("Telegram updates");
    updates.retain(|update| {
        update
            .get("update_id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id >= offset)
    });
    let result = updates.iter().cloned().collect::<Vec<_>>();
    state
        .served_update_ids
        .lock()
        .expect("served Telegram updates")
        .extend(
            result
                .iter()
                .filter_map(|update| update.get("update_id").and_then(Value::as_i64)),
        );
    Json(json!({ "ok": true, "result": result }))
}

async fn send_message(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Json<Value> {
    state
        .sent_messages
        .lock()
        .expect("Telegram sent messages")
        .push(body);
    let id = state
        .sent_messages
        .lock()
        .expect("Telegram sent messages")
        .len() as i64;
    Json(json!({ "ok": true, "result": { "message_id": id, "chat": { "id": 42 } } }))
}

async fn get_file(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Json<Value> {
    let file_id = body
        .get("file_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if file_id == "fixture-voice-file" {
        state
            .voice_file_requests
            .lock()
            .expect("voice file requests")
            .push(file_id.to_owned());
        return Json(json!({
            "ok": true,
            "result": { "file_path": "voice/input.oga", "file_size": 2575 }
        }));
    }
    state
        .file_requests
        .lock()
        .expect("Telegram getFile requests")
        .push(file_id.to_owned());
    let (path, bytes) = match file_id {
        "typed-image-file" => ("photos/native.png", native_image_png()),
        "typed-document-file" => ("documents/native.txt", NATIVE_FILE_BYTES),
        _ => return Json(json!({ "ok": false, "description": "unknown mock file" })),
    };
    Json(json!({ "ok": true, "result": { "file_path": path, "file_size": bytes.len() } }))
}

async fn download_file(
    State(state): State<Arc<MockState>>,
    path: axum::extract::Path<String>,
) -> Response {
    state
        .file_downloads
        .lock()
        .expect("Telegram file downloads")
        .push(path.0.clone());
    let (content_type, bytes) = match path.0.as_str() {
        "photos/native.png" => ("image/png", native_image_png()),
        "documents/native.txt" => ("text/plain", NATIVE_FILE_BYTES),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [(axum::http::header::CONTENT_TYPE, content_type)],
        bytes.to_vec(),
    )
        .into_response()
}

async fn ok() -> Json<Value> {
    Json(json!({ "ok": true, "result": true }))
}

async fn models() -> Json<Value> {
    Json(json!({
        "data": [{
            "id": "gpt-4o",
            "object": "model",
            "input_modalities": ["text", "image", "pdf"]
        }]
    }))
}

async fn auth_me() -> Json<Value> {
    Json(json!({ "id": "loopback-user", "email": "loopback@example.test" }))
}

async fn get_me() -> Json<Value> {
    Json(json!({ "ok": true, "result": { "id": 9, "is_bot": true, "username": "test_bot" } }))
}

async fn download_voice_file(State(state): State<Arc<MockState>>) -> impl IntoResponse {
    *state
        .voice_download_requests
        .lock()
        .expect("voice download requests") += 1;
    (
        [(axum::http::header::CONTENT_TYPE, "audio/ogg")],
        include_bytes!("../vendor/tinychannels/src/providers/telegram/fixtures/sine-ogg-opus.ogg")
            .as_slice(),
    )
}

async fn send_voice(State(state): State<Arc<MockState>>, body: Bytes) -> Json<Value> {
    state
        .sent_voice_uploads
        .lock()
        .expect("voice uploads")
        .push(body.to_vec());
    Json(json!({ "ok": true, "result": { "message_id": 700, "chat": { "id": 42 } } }))
}

async fn transcribe_voice(State(state): State<Arc<MockState>>, body: Bytes) -> Json<Value> {
    state
        .transcription_uploads
        .lock()
        .expect("STT uploads")
        .push(body.to_vec());
    Json(json!({ "text": "the dictated voice request" }))
}

async fn synthesize_voice(
    State(state): State<Arc<MockState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state.speech_requests.lock().expect("TTS requests").push(
        body.get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    );
    let audio = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        include_bytes!("../vendor/tinyvoice/crates/tinyvoice/tests/fixtures/voice-envelope.wav"),
    );
    Json(json!({ "audio_base64": audio, "audio_mime": "audio/wav" }))
}

async fn chat_completion(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Response {
    let index = {
        let mut completions = state.completions.lock().expect("model requests");
        completions.push(body.clone());
        completions.len()
    };
    let message = json!({
        "role": "assistant",
        "content": format!("mock reply {index}")
    });
    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        let mut delta = json!({ "role": "assistant" });
        delta["content"] = message["content"].clone();
        let stream = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({ "choices": [{ "index": 0, "delta": delta }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] })
        );
        return (
            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
            stream,
        )
            .into_response();
    }
    let response = json!({
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": "stop"
        }]
    });
    (StatusCode::OK, Json(response)).into_response()
}

fn message_update(update_id: i64, message_id: i64, text: &str) -> Value {
    message_update_for(update_id, message_id, 42, text)
}

fn message_update_for(update_id: i64, message_id: i64, chat_id: i64, text: &str) -> Value {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": message_id,
            "date": 1_800_000_000,
            "from": { "id": chat_id, "is_bot": false, "first_name": "Ada" },
            "chat": { "id": chat_id, "type": "private", "first_name": "Ada" },
            "text": text
        }
    })
}

fn voice_update(update_id: i64, message_id: i64) -> Value {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": message_id,
            "date": 1_800_000_000,
            "from": { "id": 42, "is_bot": false, "first_name": "Ada" },
            "chat": { "id": 42, "type": "private", "first_name": "Ada" },
            "voice": { "file_id": "fixture-voice-file", "file_unique_id": "fixture-voice", "duration": 1, "mime_type": "audio/ogg" }
        }
    })
}

fn push_update(state: &MockState, update_id: i64, message_id: i64, text: &str) {
    state
        .updates
        .lock()
        .expect("Telegram updates")
        .push_back(message_update(update_id, message_id, text));
}

fn push_update_for(state: &MockState, update_id: i64, message_id: i64, chat_id: i64, text: &str) {
    state
        .updates
        .lock()
        .expect("Telegram updates")
        .push_back(message_update_for(update_id, message_id, chat_id, text));
}

fn media_update(
    update_id: i64,
    message_id: i64,
    image: bool,
    document: bool,
    text: Option<&str>,
) -> Value {
    assert!(
        !(image && document),
        "a Telegram message has one media object"
    );
    let mut message = json!({
        "message_id": message_id,
        "date": 1_800_000_000,
        "from": { "id": 42, "is_bot": false, "first_name": "Ada" },
        "chat": { "id": 42, "type": "private", "first_name": "Ada" }
    });
    if image {
        message["photo"] = json!([{
            "file_id": "typed-image-file",
            "file_unique_id": "typed-image-unique",
            "file_size": native_image_png().len()
        }]);
    }
    if document {
        message["document"] = json!({
            "file_id": "typed-document-file",
            "file_unique_id": "typed-document-unique",
            "file_name": "native.txt",
            "mime_type": "text/plain",
            "file_size": NATIVE_FILE_BYTES.len()
        });
    }
    if let Some(text) = text {
        if image || document {
            message["caption"] = json!(text);
        } else {
            message["text"] = json!(text);
        }
    }
    json!({ "update_id": update_id, "message": message })
}

/// Where the child core loads TinyChannels (and TinyVoice) from.
///
/// CI stages the registry-pinned, digest-verified releases and names them in
/// `OPENHUMAN_CHANNELS_TEST_ARTIFACT` / `OPENHUMAN_VOICE_TEST_ARTIFACT`
/// (`scripts/ci/self-hosted/install-test-modules.sh`); the child loads those
/// files through `[[modules.overrides]]`. A developer can instead point
/// `OPENHUMAN_TEST_MODULE_SOURCE_DIR` at `scripts/modules/build.mjs` output,
/// which takes precedence. The directory must not sit under a symlink: TinyBus
/// refuses a module path with a non-directory ancestor.
enum TestModules {
    Source(std::path::PathBuf),
    Release(Vec<(&'static str, std::path::PathBuf)>),
}

fn test_modules(voice: bool) -> TestModules {
    let ids: &[&'static str] = if voice {
        &["tinychannels", "tinyvoice"]
    } else {
        &["tinychannels"]
    };
    if let Some(dir) = std::env::var_os("OPENHUMAN_TEST_MODULE_SOURCE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        for id in ids {
            assert!(
                dir.join(id).join("modules.toml").is_file(),
                "compiled {id} descriptor missing from {}",
                dir.display()
            );
        }
        return TestModules::Source(dir);
    }
    let artifacts = ids
        .iter()
        .map(|&id| {
            let var = match id {
                "tinyvoice" => "OPENHUMAN_VOICE_TEST_ARTIFACT",
                _ => "OPENHUMAN_CHANNELS_TEST_ARTIFACT",
            };
            let path = std::env::var_os(var)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    panic!(
                        "set {var} to the staged {id} release \
                         (scripts/ci/self-hosted/install-test-modules.sh), or \
                         OPENHUMAN_TEST_MODULE_SOURCE_DIR to scripts/modules/build.mjs output"
                    )
                });
            // The child runs from its own home, so a relative path must not reach it.
            let path = std::fs::canonicalize(&path)
                .unwrap_or_else(|error| panic!("{var}={}: {error}", path.display()));
            assert!(path.is_file(), "{var} names no file: {}", path.display());
            (id, path)
        })
        .collect();
    TestModules::Release(artifacts)
}

fn write_config(home: &Path, workspace: &Path, inference: &str, modules: &TestModules) {
    let openhuman_dir = home.join(".openhuman");
    std::fs::create_dir_all(&openhuman_dir).expect("create OpenHuman config dir");
    std::fs::create_dir_all(workspace).expect("create workspace");
    let mut config = format!(
        r#"default_model = "openai:gpt-4o"
chat_provider = "openai:gpt-4o"
api_url = "{inference}"
default_temperature = 0.0

[secrets]
encrypt = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[update]
enabled = false

[channels_config.telegram]
bot_token = "{BOT_TOKEN}"
allowed_users = ["42", "43"]
mention_only = false

[channel_voice]
reply_mode = "mirror"

[[cloud_providers]]
id = "p_session_restart"
slug = "openai"
label = "Session restart mock"
endpoint = "{inference}/v1"
auth_style = "none"
default_model = "gpt-4o"
"#
    );
    if let TestModules::Release(artifacts) = modules {
        for (id, path) in artifacts {
            let path = toml::Value::String(path.display().to_string());
            config.push_str(&format!(
                "\n[[modules.overrides]]\nid = \"{id}\"\npath = {path}\n"
            ));
        }
    }
    std::fs::write(openhuman_dir.join("config.toml"), &config).expect("write config");
    let parsed: openhuman_core::config::Config =
        toml::from_str(&config).expect("mock config matches the core schema");
    assert_eq!(
        parsed.channels_config.telegram.as_ref().unwrap().bot_token,
        BOT_TOKEN
    );
}

struct CoreProcess {
    child: Child,
    log_path: std::path::PathBuf,
    stopped: bool,
}

impl CoreProcess {
    fn stop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.child.kill().expect("stop core process");
        }
        self.child.wait().expect("reap core process");
        self.stopped = true;
    }
}

impl Drop for CoreProcess {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Ok(log) = std::fs::read_to_string(&self.log_path) {
                let tail = log.lines().rev().take(80).collect::<Vec<_>>();
                eprintln!(
                    "core process was dropped before the test completed; last {} log lines:\n{}",
                    tail.len(),
                    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
                );
            }
        }
    }
}

fn start_core(
    home: &Path,
    workspace: &Path,
    telegram: &str,
    inference: &str,
    modules: &TestModules,
    log_path: &Path,
    voice_enabled: bool,
) -> (CoreProcess, String) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("reserve core port")
        .local_addr()
        .expect("core port")
        .port();
    let stdout = File::create(log_path).expect("create core log");
    let stderr = stdout.try_clone().expect("clone core log handle");
    let mut command = Command::new(env!("CARGO_BIN_EXE_openhuman-core"));
    // A per-module source env var beats the config overrides; keep it out.
    command
        .env_remove("OPENHUMAN_MODULE_SOURCE_TINYCHANNELS")
        .env_remove("OPENHUMAN_MODULE_SOURCE_TINYVOICE");
    match modules {
        TestModules::Source(dir) => {
            command
                .env("OPENHUMAN_MODULE_SOURCE", "source")
                .env("OPENHUMAN_MODULE_SOURCE_DIR", dir);
        }
        TestModules::Release(_) => {
            command
                .env("OPENHUMAN_MODULE_SOURCE", "release")
                .env_remove("OPENHUMAN_MODULE_SOURCE_DIR");
        }
    }
    let child = command
        .args(["serve", "--port", &port.to_string()])
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("OPENHUMAN_WORKSPACE", workspace)
        .env("OPENHUMAN_CORE_TOKEN", RPC_TOKEN)
        .env(
            "RUST_LOG",
            "tinychannels=debug,tinychannels::remote=debug,tinychannels_module=debug,tinychannels_module::service=debug,tinystoragedrivers=debug,openhuman::storage_host=info,openhuman_core::channels=debug",
        )
        .env("OPENHUMAN_MODEL", "openai:gpt-4o")
        .env("OPENHUMAN_BACKEND_API_KEY", MOCK_API_KEY)
        .env("BACKEND_URL", inference)
        .env("OPENHUMAN_API_URL", inference)
        .env("OPENHUMAN_AUTO_UPDATE_ENABLED", "false")
        .env("OPENHUMAN_TELEGRAM_BOT_API_BASE", telegram)
        .env("OPENHUMAN_COMPONENT_CHANNELS", "1")
        .env(
            "OPENHUMAN_STORAGE_URL",
            format!("sqlite:{}", workspace.join("sessions.db").display()),
        )
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "44".repeat(32))
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .env_remove("OPENHUMAN_MODULE_PATH")
        .env_remove("OPENHUMAN_MODE")
        .env_remove("OPENHUMAN_BACKEND_SESSION_TOKEN")
        .envs(
            [
                "SANDBOX",
                "CONNECTORS",
                "HOSTING",
                "MEMES",
                "COMPUTER",
                "SEARCH",
                "DOCUMENTS",
                "WALLET",
                "JUICE",
                "VOICE",
            ]
            .map(|name| (format!("OPENHUMAN_COMPONENT_{name}"), "0")),
        )
        .env("OPENHUMAN_COMPONENT_VOICE", if voice_enabled { "1" } else { "0" })
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn openhuman-core");
    (
        CoreProcess {
            child,
            log_path: log_path.to_path_buf(),
            stopped: false,
        },
        format!("http://127.0.0.1:{port}"),
    )
}

async fn wait_until(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_new_session_reply(state: &MockState) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let (has_reply, sent_snapshot) = {
            let sent = state.sent_messages.lock().expect("Telegram sent messages");
            (
                sent.iter().any(|body| {
                    body["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("Started new session"))
                }),
                sent.clone(),
            )
        };
        if has_reply {
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for Telegram /new reply; outbound bodies: {sent_snapshot:#?}"
            );
        }
        sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_healthy(base: &str, process: &mut CoreProcess, log_path: &Path) {
    let client = reqwest::Client::new();
    let health = format!("{base}/health");
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if client
            .get(&health)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return;
        }
        if let Ok(Some(exit)) = process.child.try_wait() {
            let log = std::fs::read_to_string(log_path).unwrap_or_default();
            panic!("core exited before health became ready ({exit}):\n{log}");
        }
        assert!(Instant::now() < deadline, "core did not become healthy");
        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_completion(state: &MockState, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while state.completions.lock().expect("model requests").len() < count {
        if Instant::now() >= deadline {
            let model_requests = state.completions.lock().expect("model requests").len();
            let offsets = state
                .update_offsets
                .lock()
                .expect("Telegram offsets")
                .clone();
            let sent_messages = state
                .sent_messages
                .lock()
                .expect("Telegram sent messages")
                .clone();
            let served_update_ids = state
                .served_update_ids
                .lock()
                .expect("served Telegram updates")
                .clone();
            let file_requests = state
                .file_requests
                .lock()
                .expect("Telegram getFile requests")
                .clone();
            let file_downloads = state
                .file_downloads
                .lock()
                .expect("Telegram file downloads")
                .clone();
            panic!(
                "timed out waiting for model request {count}; observed {model_requests} model requests, Telegram offsets {offsets:?}, served updates {served_update_ids:?}, getFile requests {file_requests:?}, file downloads {file_downloads:?}, and outbound bodies {sent_messages:#?}"
            );
        }
        sleep(Duration::from_millis(50)).await;
    }
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let sent_messages = state
            .sent_messages
            .lock()
            .expect("Telegram sent messages")
            .clone();
        if sent_messages.iter().any(|body| {
            body["text"]
                .as_str()
                .is_some_and(|text| text.contains(&format!("mock reply {count}")))
        }) {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for final Telegram reply {count}; captured outbound bodies: {sent_messages:#?}"
            );
        }
        sleep(Duration::from_millis(50)).await;
    }
}

fn assert_model_received_typed_image(request: &Value) {
    use base64::Engine as _;
    let content = last_user_content(request).expect("image user content is present");
    let image = find_block_type(content, "image_url").unwrap_or_else(|| {
        fn collect_shape(value: &Value, types: &mut Vec<String>, fields: &mut Vec<String>) {
            match value {
                Value::Array(items) => {
                    for item in items {
                        collect_shape(item, types, fields);
                    }
                }
                Value::Object(object) => {
                    if let Some(kind) = object.get("type").and_then(Value::as_str) {
                        types.push(kind.to_owned());
                    }
                    for (name, item) in object {
                        fields.push(name.clone());
                        collect_shape(item, types, fields);
                    }
                }
                _ => {}
            }
        }
        let messages = request["messages"].as_array().expect("request messages");
        let shapes = messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let content = &message["content"];
                let mut types = Vec::new();
                let mut fields = Vec::new();
                collect_shape(content, &mut types, &mut fields);
                fields.sort();
                fields.dedup();
                let serialized = serde_json::to_string(content).unwrap_or_default();
                format!(
                    "message={index} role={} content_kind={} block_types={types:?} fields={fields:?} staged_ref={} image_data_url={}",
                    message["role"].as_str().unwrap_or("unknown"),
                    match content {
                        Value::Array(_) => "array",
                        Value::Object(_) => "object",
                        Value::String(_) => "string",
                        _ => "other",
                    },
                    serialized.contains("openhuman-staged://"),
                    serialized.contains("data:image/")
                )
            })
            .collect::<Vec<_>>();
        panic!("image is missing from typed model content; safe request shape: {shapes:?}");
    });
    let url = image["image_url"]["url"]
        .as_str()
        .expect("typed image carries a data URL");
    let (header, data) = url
        .split_once(',')
        .expect("typed image data URL has a payload");
    assert_eq!(header, "data:image/png;base64");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .expect("image data URL payload is base64");
    assert!(decoded.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(decoded.ends_with(b"IEND\xaeB`\x82"));
}

fn assert_model_received_file_content(request: &Value) {
    let content = last_user_content(request).expect("file user content is present");
    if let Some(document) = ["input_file", "document", "file"]
        .into_iter()
        .find_map(|kind| find_block_type(content, kind))
    {
        let encoded = serde_json::to_string(document).expect("serialize typed document");
        assert!(encoded.contains("text/plain"), "typed file MIME: {encoded}");
        let base64 = base64::engine::general_purpose::STANDARD.encode(NATIVE_FILE_BYTES);
        assert!(
            encoded.contains(&base64)
                || encoded.contains(std::str::from_utf8(NATIVE_FILE_BYTES).unwrap()),
            "typed document carries the downloaded file bytes: {encoded}"
        );
    } else {
        if !content_has_text_block(content, std::str::from_utf8(NATIVE_FILE_BYTES).unwrap()) {
            let serialized = serde_json::to_string(content).unwrap_or_default();
            panic!(
                "unsupported Document input must reach the model as extracted literal text; safe content shape: kind={} block_types={:?} staged_ref={} file_bytes_text={} file_bytes_base64={} caption={}",
                match content {
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                    Value::String(_) => "string",
                    _ => "other",
                },
                content_block_types(content),
                serialized.contains("openhuman-staged://"),
                serialized.contains(std::str::from_utf8(NATIVE_FILE_BYTES).unwrap()),
                serialized.contains(&base64::engine::general_purpose::STANDARD.encode(NATIVE_FILE_BYTES)),
                serialized.contains("file caption before media")
            );
        }
    }
    let text_blocks = collect_text_blocks(content);
    let encoded = base64::engine::general_purpose::STANDARD.encode(NATIVE_FILE_BYTES);
    assert!(text_blocks.iter().all(|text| !text.contains(&encoded)));
}

fn assert_request_retains_native_file(request: &Value) {
    let content = &request["messages"];
    if let Some(document) = ["input_file", "document", "file"]
        .into_iter()
        .find_map(|kind| find_block_type(content, kind))
    {
        let encoded = serde_json::to_string(document).expect("serialize historical document");
        assert!(encoded.contains("text/plain"));
        let base64 = base64::engine::general_purpose::STANDARD.encode(NATIVE_FILE_BYTES);
        assert!(
            encoded.contains(&base64)
                || encoded.contains(std::str::from_utf8(NATIVE_FILE_BYTES).unwrap()),
            "historical typed document retains its bytes"
        );
    } else {
        assert!(
            content_has_text_block(content, std::str::from_utf8(NATIVE_FILE_BYTES).unwrap()),
            "historical file is available as extracted literal text"
        );
    }
}

fn find_block_type<'a>(value: &'a Value, expected: &str) -> Option<&'a Value> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_block_type(item, expected)),
        Value::Object(fields) => {
            if fields.get("type").and_then(Value::as_str) == Some(expected) {
                return Some(value);
            }
            fields
                .values()
                .find_map(|item| find_block_type(item, expected))
        }
        _ => None,
    }
}

fn content_block_types(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().flat_map(content_block_types).collect(),
        Value::Object(fields) => {
            let mut kinds = fields
                .get("type")
                .and_then(Value::as_str)
                .map(|kind| vec![kind.to_owned()])
                .unwrap_or_default();
            kinds.extend(fields.values().flat_map(content_block_types));
            kinds
        }
        _ => Vec::new(),
    }
}

fn content_has_text_block(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text.contains(expected),
        Value::Array(items) => items
            .iter()
            .any(|item| content_has_text_block(item, expected)),
        Value::Object(fields) => {
            (fields.get("type").and_then(Value::as_str) == Some("text")
                && fields
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains(expected)))
                || fields
                    .values()
                    .any(|item| content_has_text_block(item, expected))
        }
        _ => false,
    }
}

fn collect_text_blocks(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text],
        Value::Array(items) => items.iter().flat_map(collect_text_blocks).collect(),
        Value::Object(fields) => {
            let mut text = Vec::new();
            if fields.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(value) = fields.get("text").and_then(Value::as_str) {
                    text.push(value);
                }
            }
            text.extend(fields.values().flat_map(collect_text_blocks));
            text
        }
        _ => Vec::new(),
    }
}

fn last_user_content(request: &Value) -> Option<&Value> {
    request["messages"]
        .as_array()?
        .iter()
        .rev()
        .find_map(|message| (message["role"] == "user").then(|| &message["content"]))
}

fn model_saw_user_text(request: &Value, text: &str) -> bool {
    request["messages"].as_array().is_some_and(|messages| {
        messages.iter().any(|message| {
            message["role"] == "user" && content_contains_text(&message["content"], text)
        })
    })
}

fn content_contains_text(content: &Value, expected: &str) -> bool {
    match content {
        Value::String(text) => text.contains(expected),
        Value::Array(parts) => parts
            .iter()
            .any(|part| content_contains_text(part, expected)),
        Value::Object(fields) => {
            fields
                .get("text")
                .is_some_and(|text| content_contains_text(text, expected))
                || fields
                    .get("content")
                    .is_some_and(|nested| content_contains_text(nested, expected))
        }
        _ => false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_telegram_session_resumes_after_the_core_process_restarts() {
    let tmp = tempfile::tempdir().expect("temporary core home");
    let home = std::fs::canonicalize(tmp.path()).expect("canonicalize temporary home");
    let workspace = home.join("workspace");
    let logs = home.join("logs");
    std::fs::create_dir_all(&logs).expect("create logs");
    let modules = test_modules(false);

    let mocks = start_mock_servers().await;
    write_config(&home, &workspace, &mocks.inference, &modules);

    // The first listener instance consumes one real native Telegram update.
    let (mut first, first_rpc) = start_core(
        &home,
        &workspace,
        &mocks.telegram,
        &mocks.inference,
        &modules,
        &logs.join("first.log"),
        false,
    );
    wait_healthy(&first_rpc, &mut first, &logs.join("first.log")).await;
    let first_pid = first.child.id();
    push_update(&mocks.state, 100, 10, FIRST_TEXT);
    wait_for_completion(&mocks.state, 1).await;
    let first_request = mocks
        .state
        .completions
        .lock()
        .expect("model requests")
        .first()
        .cloned()
        .expect("first Telegram update reached the model");
    assert!(
        model_saw_user_text(&first_request, FIRST_TEXT),
        "the first process must reach the model through native Telegram ingress: {}",
        serde_json::to_string_pretty(&first_request).unwrap()
    );
    assert_eq!(
        mocks
            .state
            .completions
            .lock()
            .expect("model requests")
            .len(),
        1,
        "the first process must finish exactly one model turn before restart"
    );
    let wait_for_update_ack = |update_id| {
        let state = Arc::clone(&mocks.state);
        async move {
            wait_until("Telegram acknowledgement of update", || {
                state
                    .update_offsets
                    .lock()
                    .expect("Telegram offsets")
                    .iter()
                    .any(|offset| *offset > update_id)
            })
            .await;
        }
    };
    wait_for_update_ack(100).await;

    // A second private conversation persists independent durable history.
    push_update_for(&mocks.state, 101, 11, 43, OTHER_CHAT_TEXT);
    wait_for_completion(&mocks.state, 2).await;
    wait_for_update_ack(101).await;
    let requests = mocks
        .state
        .completions
        .lock()
        .expect("model requests")
        .clone();
    assert!(model_saw_user_text(&requests[1], OTHER_CHAT_TEXT));
    assert!(!model_saw_user_text(&requests[1], FIRST_TEXT));

    // The real module command must reset only chat 42's durable SessionRef.
    push_update(&mocks.state, 102, 12, "/new");
    wait_for_new_session_reply(&mocks.state).await;
    wait_for_update_ack(102).await;

    push_update(&mocks.state, 103, 13, AFTER_RESET_TEXT);
    wait_for_completion(&mocks.state, 3).await;
    wait_for_update_ack(103).await;
    let reset_request = mocks
        .state
        .completions
        .lock()
        .expect("model requests")
        .get(2)
        .cloned()
        .expect("post-reset Telegram update reached the model");
    assert!(model_saw_user_text(&reset_request, AFTER_RESET_TEXT));
    assert!(!model_saw_user_text(&reset_request, FIRST_TEXT));
    assert!(!model_saw_user_text(&reset_request, OTHER_CHAT_TEXT));
    first.stop();

    // Both durable session histories survive a process restart: chat 42 keeps
    // its post-reset messages, while chat 43 retains its unrelated history.
    push_update(&mocks.state, 104, 14, RESET_AFTER_RESTART_TEXT);
    push_update_for(&mocks.state, 105, 15, 43, OTHER_AFTER_RESTART_TEXT);
    let (mut second, second_rpc) = start_core(
        &home,
        &workspace,
        &mocks.telegram,
        &mocks.inference,
        &modules,
        &logs.join("second.log"),
        false,
    );
    assert_ne!(
        first_pid,
        second.child.id(),
        "restart must use a fresh process"
    );
    wait_healthy(&second_rpc, &mut second, &logs.join("second.log")).await;
    wait_until("post-restart model requests", || {
        mocks
            .state
            .completions
            .lock()
            .expect("model requests")
            .len()
            >= 5
    })
    .await;

    let requests = mocks
        .state
        .completions
        .lock()
        .expect("model requests")
        .clone();
    assert_eq!(
        requests.len(),
        5,
        "each Telegram update must reach the model once"
    );
    let reset_chat_request = requests
        .iter()
        .find(|request| model_saw_user_text(request, RESET_AFTER_RESTART_TEXT))
        .expect("post-reset chat 42 update reached the model");
    assert!(
        model_saw_user_text(reset_chat_request, AFTER_RESET_TEXT),
        "chat 42 must resume its post-reset transcript after process restart; got {}",
        serde_json::to_string_pretty(reset_chat_request).unwrap()
    );
    assert!(!model_saw_user_text(reset_chat_request, FIRST_TEXT));
    assert!(!model_saw_user_text(reset_chat_request, OTHER_CHAT_TEXT));
    let other_chat_request = requests
        .iter()
        .find(|request| model_saw_user_text(request, OTHER_AFTER_RESTART_TEXT))
        .expect("post-restart chat 43 update reached the model");
    assert!(model_saw_user_text(other_chat_request, OTHER_CHAT_TEXT));
    assert!(!model_saw_user_text(other_chat_request, FIRST_TEXT));
    assert!(!model_saw_user_text(other_chat_request, AFTER_RESET_TEXT));
    wait_for_completion(&mocks.state, 5).await;
    second.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_telegram_typed_media_reaches_the_model_without_provider_paths() {
    let tmp = tempfile::tempdir().expect("temporary core home");
    let home = std::fs::canonicalize(tmp.path()).expect("canonicalize temporary home");
    let workspace = home.join("workspace");
    let logs = home.join("logs");
    std::fs::create_dir_all(&logs).expect("create logs");
    let modules = test_modules(false);

    let mocks = start_mock_servers().await;
    write_config(&home, &workspace, &mocks.inference, &modules);
    let (mut core, core_rpc) = start_core(
        &home,
        &workspace,
        &mocks.telegram,
        &mocks.inference,
        &modules,
        &logs.join("typed-media.log"),
        false,
    );
    wait_healthy(&core_rpc, &mut core, &logs.join("typed-media.log")).await;

    push_media_update(&mocks.state, media_update(200, 20, true, false, None));
    wait_for_media_fetch(&mocks.state, 200, "typed-image-file", "photos/native.png").await;
    wait_for_completion(&mocks.state, 1).await;
    push_media_update(&mocks.state, media_update(201, 21, false, true, None));
    wait_for_media_fetch(
        &mocks.state,
        201,
        "typed-document-file",
        "documents/native.txt",
    )
    .await;
    wait_for_completion(&mocks.state, 2).await;
    push_media_update(
        &mocks.state,
        media_update(202, 22, true, false, Some("image caption before media")),
    );
    wait_for_media_fetch(&mocks.state, 202, "typed-image-file", "photos/native.png").await;
    wait_for_completion(&mocks.state, 3).await;
    push_media_update(
        &mocks.state,
        media_update(203, 23, false, true, Some("file caption before media")),
    );
    wait_for_media_fetch(
        &mocks.state,
        203,
        "typed-document-file",
        "documents/native.txt",
    )
    .await;
    wait_for_completion(&mocks.state, 4).await;

    let requests = mocks
        .state
        .completions
        .lock()
        .expect("model requests")
        .clone();
    assert_eq!(
        requests.len(),
        4,
        "each media update reaches the model once"
    );
    assert_model_received_typed_image(&requests[0]);
    assert_model_received_file_content(&requests[1]);
    let image_mixed = serde_json::to_string(
        last_user_content(&requests[2]).expect("mixed image user message is present"),
    )
    .expect("serialize mixed image user content");
    let image_caption = image_mixed
        .find("image caption before media")
        .expect("image caption text is present");
    let image = image_mixed
        .find("image_url")
        .expect("mixed image is still a typed image block");
    assert!(image_caption < image, "image part order: {image_mixed}");

    assert_model_received_file_content(&requests[3]);
    let file_content = last_user_content(&requests[3]).expect("mixed file user message is present");
    let file_mixed = serde_json::to_string(file_content).expect("serialize mixed file content");
    let file_caption = file_mixed
        .find("file caption before media")
        .expect("file caption text is present");
    let typed_file = ["input_file", "document", "file"]
        .into_iter()
        .find_map(|kind| find_block_type(file_content, kind))
        .and_then(|document| serde_json::to_string(document).ok())
        .and_then(|document| file_mixed.find(&document));
    let file_position = typed_file.or_else(|| {
        file_mixed.find(std::str::from_utf8(NATIVE_FILE_BYTES).expect("fixture is UTF-8"))
    });
    let file_position = file_position.expect("file reaches model as a document or extracted text");
    assert!(
        file_caption < file_position,
        "file part order: {file_mixed}"
    );
    for request in &requests {
        let serialized = serde_json::to_string(request).unwrap();
        assert!(!serialized.contains("typed-image-file"));
        assert!(!serialized.contains("typed-document-file"));
        assert!(!serialized.contains("photos/native.png"));
        assert!(!serialized.contains("documents/native.txt"));
    }
    let first_pid = core.child.id();
    core.stop();

    // A new process must resolve committed native media from the same scoped
    // claims while preparing the next text-only turn.
    push_update(
        &mocks.state,
        204,
        24,
        "continue the conversation after native media restart",
    );
    let (mut restarted, restarted_rpc) = start_core(
        &home,
        &workspace,
        &mocks.telegram,
        &mocks.inference,
        &modules,
        &logs.join("typed-media-restarted.log"),
        false,
    );
    assert_ne!(
        first_pid,
        restarted.child.id(),
        "media resume uses a new process"
    );
    wait_healthy(
        &restarted_rpc,
        &mut restarted,
        &logs.join("typed-media-restarted.log"),
    )
    .await;
    wait_for_completion(&mocks.state, 5).await;
    let resumed = mocks
        .state
        .completions
        .lock()
        .expect("model requests")
        .get(4)
        .cloned()
        .expect("post-restart text reached the model");
    assert!(model_saw_user_text(
        &resumed,
        "continue the conversation after native media restart"
    ));
    assert!(find_block_type(&resumed["messages"], "image_url").is_some());
    assert_request_retains_native_file(&resumed);
    restarted.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(feature = "voice")]
async fn telegram_voice_note_runs_scoped_stt_agent_and_mirror_tts_through_modules() {
    let tmp = tempfile::tempdir().expect("temporary core home");
    let home = std::fs::canonicalize(tmp.path()).expect("canonicalize temporary home");
    let workspace = home.join("workspace");
    let logs = home.join("logs");
    std::fs::create_dir_all(&logs).expect("create logs");
    let modules = test_modules(true);

    let mocks = start_mock_servers().await;
    write_config(&home, &workspace, &mocks.inference, &modules);
    let (mut core, rpc) = start_core(
        &home,
        &workspace,
        &mocks.telegram,
        &mocks.inference,
        &modules,
        &logs.join("voice.log"),
        true,
    );
    wait_healthy(&rpc, &mut core, &logs.join("voice.log")).await;
    mocks
        .state
        .updates
        .lock()
        .expect("Telegram updates")
        .push_back(voice_update(200, 20));

    wait_until("voice note reaches inference", || {
        !mocks
            .state
            .completions
            .lock()
            .expect("model requests")
            .is_empty()
    })
    .await;

    let requests = mocks
        .state
        .completions
        .lock()
        .expect("model requests")
        .clone();
    assert_eq!(
        requests.len(),
        1,
        "one inbound voice update produces one agent turn"
    );
    assert!(
        model_saw_user_text(&requests[0], "the dictated voice request"),
        "the model receives the transcript produced by the scoped cloud STT callback; file requests={:?}, downloads={}, transcription uploads={}",
        mocks
            .state
            .voice_file_requests
            .lock()
            .expect("voice file requests"),
        mocks
            .state
            .voice_download_requests
            .lock()
            .expect("voice download requests"),
        mocks
            .state
            .transcription_uploads
            .lock()
            .expect("STT uploads")
            .len()
    );
    assert_eq!(
        mocks
            .state
            .transcription_uploads
            .lock()
            .expect("STT uploads")
            .len(),
        1,
        "Telegram voice media is downloaded and transcribed once"
    );
    assert!(
        mocks
            .state
            .transcription_uploads
            .lock()
            .expect("STT uploads")[0]
            .windows(4)
            .any(|window| window == b"OggS"),
        "the cloud STT mock receives the downloaded audio multipart payload"
    );
    assert_eq!(
        mocks
            .state
            .voice_file_requests
            .lock()
            .expect("voice file requests")
            .as_slice(),
        &["fixture-voice-file"],
        "the real Telegram voice file ID is resolved before scoped transcription"
    );
    wait_until("mirror reply reaches Telegram sendVoice", || {
        !mocks
            .state
            .sent_voice_uploads
            .lock()
            .expect("voice uploads")
            .is_empty()
    })
    .await;
    assert_eq!(
        mocks
            .state
            .speech_requests
            .lock()
            .expect("TTS requests")
            .as_slice(),
        &["mock reply 1"]
    );
    assert!(
        mocks
            .state
            .sent_voice_uploads
            .lock()
            .expect("voice uploads")[0]
            .windows(4)
            .any(|window| window == b"OggS"),
        "TinyVoice converts the WAV TTS response to Ogg before Telegram sendVoice"
    );
    let voice_log = std::fs::read_to_string(logs.join("voice.log"))
        .expect("read captured core log after voice delivery");
    assert!(
        voice_log.contains("[voice message]"),
        "captured core log should retain the content-free voice preview"
    );
    assert!(
        !voice_log.contains("the dictated voice request"),
        "voice transcripts must not be written to the core log"
    );
    core.stop();
}

fn push_media_update(state: &MockState, update: Value) {
    state
        .updates
        .lock()
        .expect("Telegram updates")
        .push_back(update);
}

async fn wait_for_media_fetch(state: &MockState, update_id: i64, file_id: &str, file_path: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let served = state
            .served_update_ids
            .lock()
            .expect("served Telegram updates")
            .contains(&update_id);
        let metadata = state
            .file_requests
            .lock()
            .expect("Telegram getFile requests")
            .iter()
            .any(|request| request == file_id);
        let downloaded = state
            .file_downloads
            .lock()
            .expect("Telegram file downloads")
            .iter()
            .any(|download| download == file_path);
        assert!(
            Instant::now() < deadline,
            "media update {update_id} did not reach file fetch: served={served}, getFile={metadata}, downloaded={downloaded}"
        );
        if served && metadata && downloaded {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
}
