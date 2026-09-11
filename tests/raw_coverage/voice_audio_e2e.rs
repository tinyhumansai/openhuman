//! JSON-RPC E2E coverage for the 14 uncovered `voice` controllers and all three
//! `audio_toolkit` controllers (the latter namespace was at 0%).
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target;
//! `build.rs` globs `tests/raw_coverage/` and generates the `mod` list. That
//! target already declares `required-features = ["voice", "inference"]`, both of
//! which are in `scripts/ci/product-features.sh`, so no `Cargo.toml` edit is
//! needed for the gates these controllers sit behind.
//!
//! Run with:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)"`
//!
//! # How the cloud-backed voice paths stay hermetic
//!
//! `voice_tts_dispatch`, `voice_reply_synthesize`, `voice_stt_dispatch`,
//! `voice_transcribe_bytes` and `voice_agent_signed_url` all route through the
//! hosted backend behind a session token. Rather than skip them, this file
//! stands up a **mock backend** on loopback (`/auth/me`,
//! `/openai/v1/audio/speech`, `/openai/v1/audio/transcriptions`,
//! `/voice-agent/get-signed-url`), points `api_url` at it, and mints a session
//! through the real `openhuman.auth_store_session` RPC. `ensure_secure_backend_url`
//! permits a loopback `http://` origin, which is what makes that possible.
//!
//! `voice_tts` is the one path that does not go to the backend — it drives the
//! local Piper binary. A stub `piper` on `PIPER_BIN` plus a stub voice model
//! makes its happy path assertable without a 60 MB ONNX download.
//!
//! `voice_server_start` deliberately uses a **non-`fn` hotkey**: on macOS every
//! other key is refused before any OS event tap is created (#2677), so the case
//! can exercise the lifecycle without a test grabbing the developer's keyboard.
//!
//! Env is process-global and every aggregated suite shares one process, so each
//! case takes the **crate-wide** [`env_lock`] for its whole body.

#![cfg(feature = "voice")]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "voice-audio-e2e-token";
const SESSION_JWT: &str = "voice-audio-e2e-jwt";
/// One second of silence as an mp3-shaped payload. The content does not have to
/// decode — nothing in the path under test parses the audio — but it must be
/// non-empty, because `transcribe_cloud` rejects an empty decode.
const FAKE_MP3_BASE64: &str = "SUQzBAAAAAAAI1RTU0UAAAAPAAADTGF2ZjU4Ljc2LjEwMAAAAAAAAAAAAAAA";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// Crate-wide, not file-local: all aggregated suites share one process, so a
/// private mutex would not mutually exclude with anyone else's env mutation.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvVarGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// See `sandbox_runtime_platform_e2e.rs`: `core::auth::RPC_TOKEN` is a
/// process-global `OnceLock`, so inside the aggregated binary the first suite
/// to initialise pins the bearer for every later one. Use the token this
/// process actually validates rather than assuming ours won.
fn ensure_rpc_auth() -> String {
    AUTH_INIT.get_or_init(|| {
        if std::env::var(CORE_TOKEN_ENV_VAR)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .is_none()
        {
            std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        }
        let token_dir = std::env::temp_dir().join("openhuman-voice-audio-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    get_rpc_token()
        .expect("core RPC token must be initialised before serving")
        .to_string()
}

// ── mock backend ───────────────────────────────────────────────────────────

fn unauthorized() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "success": false, "error": "missing or invalid bearer" })),
    )
}

fn require_session(headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    let actual = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    match actual {
        Some(value) if value == format!("Bearer {SESSION_JWT}") => Ok(()),
        _ => Err(unauthorized()),
    }
}

async fn mock_current_user(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    Ok(Json(json!({
        "success": true,
        "data": { "_id": "voice-e2e-user", "username": "voice-e2e" }
    })))
}

/// The hosted TTS proxy. Echoes back the requested voice so a case can prove
/// the *routing* decision reached the backend, not just that a call happened.
async fn mock_audio_speech(
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    let text = body.get("text").and_then(Value::as_str).unwrap_or_default();
    if text.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": "text is required" })),
        ));
    }
    Ok(Json(json!({
        "audio_base64": FAKE_MP3_BASE64,
        "audio_mime": "audio/mpeg",
        "visemes": [
            { "viseme": "AA", "start_ms": 0, "end_ms": 120 },
            { "viseme": "MM", "start_ms": 120, "end_ms": 240 }
        ],
        // Echoed for assertion; the client ignores unknown fields.
        "echo_voice_id": body.get("voice_id").cloned().unwrap_or(Value::Null),
        "echo_text": text,
    })))
}

/// The hosted STT proxy. Multipart in, `{ "text": … }` out.
///
/// The body is scanned as raw bytes rather than parsed: this crate's `axum` is
/// built without the `multipart` feature (see `Cargo.toml`), and a mock only
/// needs to prove the client actually sent a file part and which model it
/// asked for.
async fn mock_audio_transcriptions(
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    let raw = String::from_utf8_lossy(&body);
    if !raw.contains("name=\"file\"") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": "no audio part" })),
        ));
    }
    // The `model` part is `…name="model"\r\n\r\n<value>\r\n--boundary`.
    let model = raw
        .split("name=\"model\"")
        .nth(1)
        .and_then(|rest| rest.split("\r\n\r\n").nth(1))
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(Json(json!({ "text": format!("transcribed by {model}") })))
}

async fn mock_voice_agent_signed_url(
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    Ok(Json(json!({
        "success": true,
        "data": {
            "signedUrl": "wss://voice.e2e.test/convai?token=e2e-signed",
            "agentId": "agent_e2e_001"
        }
    })))
}

fn mock_backend_router() -> Router {
    Router::new()
        .route("/auth/me", get(mock_current_user))
        .route("/settings", get(mock_current_user))
        .route("/openai/v1/audio/speech", post(mock_audio_speech))
        .route(
            "/openai/v1/audio/transcriptions",
            post(mock_audio_transcriptions),
        )
        .route(
            "/voice-agent/get-signed-url",
            get(mock_voice_agent_signed_url),
        )
}

async fn serve(
    router: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

// ── harness ────────────────────────────────────────────────────────────────

/// `extra` is spliced in **above the first table** so a caller may pass either a
/// bare top-level key or a whole `[table]`; appending it at the end would make a
/// top-level key land inside `[memory_tree]`.
///
/// `[local_ai]` is deliberately *not* in the fixed template: the local-Piper
/// case needs `runtime_enabled = true`, and declaring the table twice in one
/// document is a TOML duplicate-key error. `runtime_enabled` defaults to false,
/// which is what every other case wants anyway.
fn write_config(dir: &Path, api_url: &str, extra: &str) {
    std::fs::create_dir_all(dir).expect("create config dir");
    let cfg = format!(
        r#"api_url = "{api_url}"
default_model = "e2e-model"
default_temperature = 0.2
{extra}
[secrets]
encrypt = false

[modules]
enabled = false
allow_download = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
"#
    );
    std::fs::write(dir.join("config.toml"), &cfg).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("test config must match schema");
}

struct TestHarness {
    tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    token: String,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    mock_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl TestHarness {
    fn home(&self) -> &Path {
        self.tmp.path()
    }

    fn workspace(&self) -> std::path::PathBuf {
        self.tmp.path().join("workspace")
    }

    async fn rpc(&self, id: i64, method: &str, params: Value) -> Value {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("client");
        let url = format!("{}/rpc", self.rpc_base.trim_end_matches('/'));
        let response = client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "HTTP transport should accept {method}"
        );
        response
            .json::<Value>()
            .await
            .unwrap_or_else(|err| panic!("json for {method}: {err}"))
    }

    /// Mint a backend session through the real auth RPC, which validates the
    /// JWT against the mock's `GET /auth/me`. Every backend-bound voice call
    /// below needs this to have happened.
    async fn store_session(&self, id: i64) {
        let stored = self
            .rpc(
                id,
                "openhuman.auth_store_session",
                json!({ "token": SESSION_JWT, "user_id": "voice-e2e-user" }),
            )
            .await;
        assert!(
            stored.get("error").is_none(),
            "storing the e2e session must succeed: {stored}"
        );
    }

    fn shutdown(self) {
        self.rpc_join.abort();
        self.mock_join.abort();
    }
}

async fn setup(extra: &str) -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let (mock_addr, mock_join) = serve(mock_backend_router()).await;
    let mock_origin = format!("http://127.0.0.1:{}", mock_addr.port());

    write_config(&home.join(".openhuman"), &mock_origin, extra);
    // `auth_store_session` activates the user and reloads config from the
    // user-scoped directory; without a copy there it would fall back to
    // defaults and lose the mock `api_url`.
    write_config(
        &home.join(".openhuman").join("users").join("voice-e2e-user"),
        &mock_origin,
        extra,
    );

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("PIPER_BIN"),
        EnvVarGuard::unset("OPENHUMAN_TEST_REPLY_SPEECH_SEAM"),
        EnvVarGuard::unset("OPENHUMAN_EMAIL_CAPTURE_DIR"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
    ];

    let token = ensure_rpc_auth();
    let (rpc_addr, rpc_join) = serve(build_core_http_router(false)).await;

    TestHarness {
        tmp,
        _guards: guards,
        rpc_base: format!("http://{rpc_addr}"),
        token,
        rpc_join,
        mock_join,
    }
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

fn err_message(value: &Value, context: &str) -> String {
    let error = value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"));
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error without message: {error}"))
        .to_string()
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

/// A stub `piper` that writes a minimal RIFF/WAVE file to whatever
/// `--output_file` names, so the local TTS path can be driven end to end.
#[cfg(unix)]
fn write_piper_stub(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).expect("create stub dir");
    let path = dir.join("piper");
    std::fs::write(
        &path,
        r#"#!/bin/sh
# Mirrors piper's CLI shape: read text on stdin, write a wav to --output_file.
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --output_file) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cat > /dev/null
[ -n "$out" ] || exit 2
printf 'RIFF$\000\000\000WAVEfmt \020\000\000\000' > "$out"
printf '\001\000\001\000\200>\000\000\000}\000\000\002\000\020\000data\000\000\000\000' >> "$out"
"#,
    )
    .expect("write piper stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
    path
}

// ── voice: provider settings ───────────────────────────────────────────────

#[tokio::test]
async fn voice_set_providers_persists_and_validates_the_selectors() {
    let _lock = env_lock();
    let harness = setup("").await;

    let set = harness
        .rpc(
            80_001,
            "openhuman.voice_set_providers",
            json!({
                "stt_provider": "cloud",
                "tts_provider": "piper",
                "stt_model": "whisper-1",
                "tts_voice": "en_US-amy-medium"
            }),
        )
        .await;
    let set = payload(&set, "voice_set_providers");
    assert_eq!(set.get("stt_provider"), Some(&json!("cloud")));
    assert_eq!(set.get("tts_provider"), Some(&json!("piper")));
    assert_eq!(set.get("stt_model_id"), Some(&json!("whisper-1")));
    assert_eq!(set.get("tts_voice_id"), Some(&json!("en_US-amy-medium")));

    // Persisted, not merely echoed: read it back through a different RPC.
    let status = harness
        .rpc(80_002, "openhuman.voice_status", json!({}))
        .await;
    let status = payload(&status, "voice_status after set_providers");
    assert_eq!(
        status.get("tts_provider"),
        Some(&json!("piper")),
        "the selector must survive into the status surface: {status}"
    );

    // Omitted fields are left alone — a settings panel that only changes the
    // TTS dropdown must not blank the STT model.
    let partial = harness
        .rpc(
            80_003,
            "openhuman.voice_set_providers",
            json!({ "tts_provider": "cloud" }),
        )
        .await;
    let partial = payload(&partial, "voice_set_providers partial");
    assert_eq!(partial.get("tts_provider"), Some(&json!("cloud")));
    assert_eq!(
        partial.get("stt_model_id"),
        Some(&json!("whisper-1")),
        "an omitted field must keep its current value: {partial}"
    );
    assert_eq!(partial.get("stt_provider"), Some(&json!("cloud")));

    // Failure path: the removed local whisper engine is rejected rather than
    // silently remapped — accepting it would undo the config migration.
    let removed = harness
        .rpc(
            80_004,
            "openhuman.voice_set_providers",
            json!({ "stt_provider": "whisper" }),
        )
        .await;
    let message = err_message(&removed, "voice_set_providers whisper");
    assert!(
        message.contains("was removed with the bundled whisper.cpp engine"),
        "'whisper' must be refused with the migration explanation: {message}"
    );

    let removed_local = harness
        .rpc(
            80_005,
            "openhuman.voice_set_providers",
            json!({ "stt_provider": "local" }),
        )
        .await;
    assert!(
        err_message(&removed_local, "voice_set_providers local")
            .contains("was removed with the bundled whisper.cpp engine"),
        "'local' is the same removed engine and must be refused too"
    );

    // …and the rejected write must not have landed.
    let after = harness
        .rpc(80_006, "openhuman.voice_status", json!({}))
        .await;
    assert_ne!(
        payload(&after, "voice_status after rejection").get("stt_engine"),
        Some(&json!("whisper")),
        "a refused provider must never reach the config"
    );

    harness.shutdown();
}

#[tokio::test]
async fn voice_update_provider_settings_registers_providers_and_rejects_bad_enums() {
    let _lock = env_lock();
    let harness = setup("").await;

    let updated = harness
        .rpc(
            81_001,
            "openhuman.voice_update_provider_settings",
            json!({
                "voice_providers": [{
                    "slug": "deepgram",
                    "label": "Deepgram",
                    "endpoint": "https://api.deepgram.com/v1",
                    "auth_style": "bearer",
                    "capability": "stt",
                    "stt_api_style": "deepgram",
                    "default_stt_model": "nova-2"
                }],
                "stt_provider": "deepgram:nova-2"
            }),
        )
        .await;
    let updated = payload(&updated, "voice_update_provider_settings");
    let providers = updated
        .get("voice_providers")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("voice_providers must be an array: {updated}"));
    assert_eq!(providers.len(), 1, "exactly the one entry sent: {updated}");
    let entry = &providers[0];
    assert_eq!(entry.get("slug"), Some(&json!("deepgram")));
    assert_eq!(entry.get("label"), Some(&json!("Deepgram")));
    assert_eq!(
        entry.get("endpoint"),
        Some(&json!("https://api.deepgram.com/v1"))
    );
    assert_eq!(entry.get("capability"), Some(&json!("stt")));
    assert_eq!(entry.get("default_stt_model"), Some(&json!("nova-2")));
    assert!(
        entry
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty()),
        "an entry with no id must be assigned one: {entry}"
    );
    assert_eq!(updated.get("stt_provider"), Some(&json!("deepgram:nova-2")));

    let assigned_id = entry
        .get("id")
        .and_then(Value::as_str)
        .expect("id")
        .to_string();

    // list_models resolves by slug *or* id and returns the catalogue the
    // settings dropdown renders.
    for (id, key) in [(81_002, "deepgram"), (81_003, assigned_id.as_str())] {
        let models = harness
            .rpc(
                id,
                "openhuman.voice_list_models",
                json!({ "provider_id": key, "capability": "stt" }),
            )
            .await;
        let models = payload(&models, "voice_list_models deepgram")
            .get("models")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("models must be an array for {key}"))
            .clone();
        assert_eq!(
            models.len(),
            6,
            "the deepgram STT catalogue is six entries: {models:?}"
        );
        assert_eq!(
            models[0].get("id"),
            Some(&json!("nova-2")),
            "the recommended model must lead the list: {models:?}"
        );
        assert!(
            models
                .iter()
                .all(|m| m.get("label").and_then(Value::as_str).is_some()),
            "every entry must carry a display label: {models:?}"
        );
    }

    // Deepgram has no TTS catalogue — asking for one must return empty, not
    // the STT list under a TTS heading.
    let tts_models = harness
        .rpc(
            81_004,
            "openhuman.voice_list_models",
            json!({ "provider_id": "deepgram", "capability": "tts" }),
        )
        .await;
    assert_eq!(
        payload(&tts_models, "voice_list_models deepgram tts").get("models"),
        Some(&json!([])),
        "an STT-only provider must not advertise voices"
    );

    // An unregistered provider yields an empty catalogue rather than an error.
    let unknown = harness
        .rpc(
            81_005,
            "openhuman.voice_list_models",
            json!({ "provider_id": "not-registered" }),
        )
        .await;
    assert_eq!(
        payload(&unknown, "voice_list_models unknown").get("models"),
        Some(&json!([]))
    );

    // Failure path: `provider_id` is required.
    let missing = harness
        .rpc(81_006, "openhuman.voice_list_models", json!({}))
        .await;
    assert!(
        err_message(&missing, "voice_list_models missing")
            .contains("missing required param 'provider_id'"),
        "an omitted provider_id must be refused at the dispatch boundary"
    );

    // Failure path: a reserved slug cannot be claimed by a user-defined
    // provider — that would let a BYO entry shadow the managed engine.
    let reserved = harness
        .rpc(
            81_007,
            "openhuman.voice_update_provider_settings",
            json!({ "voice_providers": [{ "slug": "cloud" }] }),
        )
        .await;
    assert!(
        err_message(&reserved, "voice_update_provider_settings reserved")
            .contains("is reserved and cannot be used"),
        "the reserved-slug guard must hold"
    );

    // Failure path: each enum is validated with a message naming the valid set.
    for (id, field, value, expected) in [
        (81_010, "capability", "audio", "invalid capability 'audio'"),
        (81_011, "auth_style", "basic", "invalid auth_style 'basic'"),
        (
            81_012,
            "stt_api_style",
            "whisper",
            "invalid stt_api_style 'whisper'",
        ),
        (
            81_013,
            "tts_api_style",
            "azure",
            "invalid tts_api_style 'azure'",
        ),
    ] {
        // Built as a Map rather than inline in `json!` because the key is a
        // loop variable, not a literal.
        let mut provider = serde_json::Map::new();
        provider.insert("slug".to_string(), json!("acme"));
        provider.insert(field.to_string(), json!(value));
        let bad = harness
            .rpc(
                id,
                "openhuman.voice_update_provider_settings",
                json!({ "voice_providers": [Value::Object(provider)] }),
            )
            .await;
        let message = err_message(&bad, &format!("voice_update_provider_settings {field}"));
        assert!(
            message.contains(expected),
            "{field}={value} must be refused by name, got: {message}"
        );
    }

    // …and none of those refusals may have clobbered the registered provider.
    let survived = harness
        .rpc(
            81_020,
            "openhuman.voice_update_provider_settings",
            json!({}),
        )
        .await;
    let survived = payload(&survived, "voice_update_provider_settings no-op")
        .get("voice_providers")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("voice_providers must be an array"))
        .clone();
    assert_eq!(
        survived.len(),
        1,
        "a rejected update must be atomic — deepgram must still be registered: {survived:?}"
    );
    assert_eq!(survived[0].get("slug"), Some(&json!("deepgram")));

    harness.shutdown();
}

// ── voice: server lifecycle ────────────────────────────────────────────────

#[tokio::test]
async fn voice_server_status_start_and_stop_report_a_consistent_state_machine() {
    let _lock = env_lock();
    let harness = setup("").await;

    // Before anything is started the status surface must still answer — a
    // settings panel that cannot render until a server exists is broken.
    let idle = harness
        .rpc(82_001, "openhuman.voice_server_status", json!({}))
        .await;
    let idle = payload(&idle, "voice_server_status cold");
    assert!(
        idle.get("state").and_then(Value::as_str).is_some(),
        "status must always carry a state: {idle}"
    );
    for field in ["state", "hotkey", "activation_mode", "transcription_count"] {
        assert!(
            idle.get(field).is_some(),
            "voice server status is missing `{field}`: {idle}"
        );
    }

    // Stopping a server that was never started reports stopped rather than
    // erroring — the UI's stop button must be idempotent.
    let stopped = harness
        .rpc(82_002, "openhuman.voice_server_stop", json!({}))
        .await;
    let stopped = payload(&stopped, "voice_server_stop cold");
    assert_eq!(
        stopped.get("state"),
        Some(&json!("stopped")),
        "stopping an unstarted server must report stopped: {stopped}"
    );
    assert_eq!(stopped.get("transcription_count"), Some(&json!(0)));

    // Start with an explicit, deliberately non-`fn` hotkey. On macOS every
    // non-`fn` key is refused before any event tap is created (#2677), so this
    // exercises the lifecycle without a test capturing global input.
    let started = harness
        .rpc(
            82_003,
            "openhuman.voice_server_start",
            json!({ "hotkey": "ctrl+alt+shift+f9", "activation_mode": "tap", "skip_cleanup": true }),
        )
        .await;
    let started = payload(&started, "voice_server_start");
    assert_eq!(
        started.get("hotkey"),
        Some(&json!("ctrl+alt+shift+f9")),
        "the requested hotkey must be reflected in the status: {started}"
    );
    assert_eq!(
        started.get("activation_mode"),
        Some(&json!("tap")),
        "the requested activation mode must be reflected: {started}"
    );
    let state = started
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert!(
        matches!(
            state.as_str(),
            "stopped" | "idle" | "recording" | "processing"
        ),
        "state must be one of the declared variants, got: {state}"
    );

    // Whatever the host allowed, the singleton is now initialised and both
    // read RPCs must agree with each other.
    let after = harness
        .rpc(82_004, "openhuman.voice_server_status", json!({}))
        .await;
    let after = payload(&after, "voice_server_status after start");
    assert_eq!(
        after.get("hotkey"),
        started.get("hotkey"),
        "status and start must describe the same server: {after}"
    );

    // A start with a *different* config while running must be refused rather
    // than silently rebinding the user's hotkey under them. Only assert that
    // when the server actually came up — on a host that refused the listener
    // there is nothing running to conflict with.
    if state != "stopped" {
        let conflicting = harness
            .rpc(
                82_005,
                "openhuman.voice_server_start",
                json!({ "hotkey": "ctrl+alt+shift+f10", "activation_mode": "push" }),
            )
            .await;
        assert!(
            err_message(&conflicting, "voice_server_start conflicting").contains("stop it first"),
            "a conflicting start must refuse and say how to resolve it"
        );
    }

    // Always leave the singleton stopped — another suite shares this process.
    let final_stop = harness
        .rpc(82_006, "openhuman.voice_server_stop", json!({}))
        .await;
    assert_eq!(
        payload(&final_stop, "voice_server_stop final").get("state"),
        Some(&json!("stopped")),
        "stop must land the server in the stopped state"
    );

    harness.shutdown();
}

// ── voice: overlay STT notify ──────────────────────────────────────────────

#[tokio::test]
async fn voice_overlay_stt_notify_accepts_each_state_and_requires_text_for_completion() {
    let _lock = env_lock();
    let harness = setup("").await;

    for (id, state) in [
        (83_001, "recording_started"),
        (83_002, "cancelled"),
        (83_003, "error"),
    ] {
        let notified = harness
            .rpc(
                id,
                "openhuman.voice_overlay_stt_notify",
                json!({ "state": state }),
            )
            .await;
        assert_eq!(
            payload(&notified, &format!("overlay_stt_notify {state}")).get("ok"),
            Some(&json!(true)),
            "state '{state}' must be accepted without text"
        );
    }

    let done = harness
        .rpc(
            83_004,
            "openhuman.voice_overlay_stt_notify",
            json!({ "state": "transcription_done", "text": "hello from the overlay" }),
        )
        .await;
    assert_eq!(
        payload(&done, "overlay_stt_notify done").get("ok"),
        Some(&json!(true))
    );

    // Failure path: a completion with no transcript is meaningless and must be
    // refused rather than publishing an empty transcription into the composer.
    let no_text = harness
        .rpc(
            83_005,
            "openhuman.voice_overlay_stt_notify",
            json!({ "state": "transcription_done" }),
        )
        .await;
    assert!(
        err_message(&no_text, "overlay_stt_notify no text")
            .contains("`text` is required for transcription_done"),
        "a completion without a transcript must be refused"
    );

    // Failure path: an unknown state.
    let bad_state = harness
        .rpc(
            83_006,
            "openhuman.voice_overlay_stt_notify",
            json!({ "state": "listening" }),
        )
        .await;
    assert!(
        err_message(&bad_state, "overlay_stt_notify bad state").contains("invalid params"),
        "an unrecognised state must be a params error"
    );

    // Failure path: `state` is required.
    let missing = harness
        .rpc(83_007, "openhuman.voice_overlay_stt_notify", json!({}))
        .await;
    assert!(
        err_message(&missing, "overlay_stt_notify missing")
            .contains("missing required param 'state'"),
        "an omitted state must be refused at the dispatch boundary"
    );

    harness.shutdown();
}

// ── voice: backend-bound synthesis and transcription ───────────────────────

#[tokio::test]
async fn voice_tts_dispatch_and_reply_synthesize_route_through_the_configured_provider() {
    let _lock = env_lock();
    let harness = setup("").await;

    // Every path here needs a session, and the "no session" branch is worth
    // asserting first — it is the state a signed-out user is in.
    let unauthenticated = harness
        .rpc(
            84_001,
            "openhuman.voice_tts_dispatch",
            json!({ "text": "hello", "provider": "cloud" }),
        )
        .await;
    assert!(
        err_message(&unauthenticated, "voice_tts_dispatch no session")
            .contains("no backend session token"),
        "a signed-out user must be told to sign in, not given an opaque failure"
    );

    harness.store_session(84_002).await;

    // tts_dispatch with an explicit provider + voice.
    let dispatched = harness
        .rpc(
            84_003,
            "openhuman.voice_tts_dispatch",
            json!({ "text": "hello from e2e", "provider": "cloud", "voice": "voice_abc" }),
        )
        .await;
    let dispatched = payload(&dispatched, "voice_tts_dispatch");
    assert_eq!(
        dispatched.get("audio_mime"),
        Some(&json!("audio/mpeg")),
        "the backend's mime must be carried through verbatim: {dispatched}"
    );
    assert_eq!(
        dispatched.get("audio_base64"),
        Some(&json!(FAKE_MP3_BASE64)),
        "the audio payload must reach the caller unmodified: {dispatched}"
    );
    let visemes = dispatched
        .get("visemes")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("visemes must be an array: {dispatched}"));
    assert_eq!(
        visemes.len(),
        2,
        "both viseme frames must be normalised, not dropped: {dispatched}"
    );
    assert_eq!(visemes[0].get("viseme"), Some(&json!("AA")));
    assert_eq!(visemes[1].get("end_ms"), Some(&json!(240)));

    // reply_synthesize honours the *configured* provider rather than always
    // hitting the cloud proxy. Pin it to cloud, then prove the voice id the
    // caller passed reached the backend.
    let _ = harness
        .rpc(
            84_004,
            "openhuman.voice_set_providers",
            json!({ "tts_provider": "cloud" }),
        )
        .await;
    let synthesized = harness
        .rpc(
            84_005,
            "openhuman.voice_reply_synthesize",
            json!({ "text": "spoken reply", "voice_id": "voice_reply_42" }),
        )
        .await;
    let synthesized = payload(&synthesized, "voice_reply_synthesize");
    assert_eq!(synthesized.get("audio_mime"), Some(&json!("audio/mpeg")));
    assert_eq!(
        synthesized.get("audio_base64"),
        Some(&json!(FAKE_MP3_BASE64))
    );

    // Failure path: the backend rejects empty text, and that rejection must
    // surface rather than being swallowed into a silent empty clip.
    let empty = harness
        .rpc(
            84_006,
            "openhuman.voice_reply_synthesize",
            json!({ "text": "   " }),
        )
        .await;
    assert!(
        err_message(&empty, "voice_reply_synthesize empty").contains("text is required"),
        "empty text must be refused"
    );

    // Failure path: `text` is required by the param schema.
    let missing = harness
        .rpc(84_007, "openhuman.voice_tts_dispatch", json!({}))
        .await;
    assert!(
        err_message(&missing, "voice_tts_dispatch missing text")
            .contains("missing required param 'text'"),
        "an omitted `text` must be refused at the dispatch boundary"
    );

    // Failure path: an unregistered provider slug cannot be dispatched to.
    let unregistered = harness
        .rpc(
            84_008,
            "openhuman.voice_tts_dispatch",
            json!({ "text": "hi", "provider": "not-a-provider" }),
        )
        .await;
    assert!(
        err_message(&unregistered, "voice_tts_dispatch unregistered")
            .contains("no voice provider with slug 'not-a-provider'"),
        "dispatching to an unknown slug must name the slug"
    );

    harness.shutdown();
}

#[tokio::test]
async fn voice_stt_dispatch_and_transcribe_bytes_reach_the_backend_transcription_proxy() {
    let _lock = env_lock();
    let harness = setup("").await;

    // Signed-out first.
    let unauthenticated = harness
        .rpc(
            85_001,
            "openhuman.voice_stt_dispatch",
            json!({ "audio_base64": FAKE_MP3_BASE64, "provider": "cloud" }),
        )
        .await;
    assert!(
        err_message(&unauthenticated, "voice_stt_dispatch no session")
            .contains("no backend session token"),
        "a signed-out transcription must say to sign in"
    );

    harness.store_session(85_002).await;

    let dispatched = harness
        .rpc(
            85_003,
            "openhuman.voice_stt_dispatch",
            json!({
                "audio_base64": FAKE_MP3_BASE64,
                "provider": "cloud",
                "mime_type": "audio/mpeg",
                "file_name": "clip.mp3",
                "language": "en"
            }),
        )
        .await;
    let dispatched = payload(&dispatched, "voice_stt_dispatch");
    assert_eq!(
        dispatched.get("provider"),
        Some(&json!("cloud")),
        "the dispatch reply must name the provider that served it: {dispatched}"
    );
    let text = dispatched
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        text.starts_with("transcribed by "),
        "the backend's transcript must reach the caller: {dispatched}"
    );

    // Failure paths on the decode guard — these run before any network call,
    // so a malformed clip never costs the user an upload.
    let not_base64 = harness
        .rpc(
            85_004,
            "openhuman.voice_stt_dispatch",
            json!({ "audio_base64": "!!!not base64!!!", "provider": "cloud" }),
        )
        .await;
    assert!(
        err_message(&not_base64, "voice_stt_dispatch bad base64").contains("invalid base64 audio"),
        "an undecodable payload must be rejected before upload"
    );

    let empty = harness
        .rpc(
            85_005,
            "openhuman.voice_stt_dispatch",
            json!({ "audio_base64": "", "provider": "cloud" }),
        )
        .await;
    assert!(
        err_message(&empty, "voice_stt_dispatch empty").contains("audio_base64 is required"),
        "an empty payload must be refused"
    );

    // transcribe_bytes takes raw bytes and an extension. Its extension guard
    // is the thing worth pinning: it becomes a temp file name.
    let bad_ext = harness
        .rpc(
            85_006,
            "openhuman.voice_transcribe_bytes",
            json!({ "audio_bytes": [1, 2, 3], "extension": "wav/../../etc" }),
        )
        .await;
    assert!(
        err_message(&bad_ext, "voice_transcribe_bytes bad ext").contains("must be alphanumeric"),
        "a non-alphanumeric extension must be refused — it names a temp file"
    );

    let blank_ext = harness
        .rpc(
            85_007,
            "openhuman.voice_transcribe_bytes",
            json!({ "audio_bytes": [1, 2, 3], "extension": "." }),
        )
        .await;
    assert!(
        err_message(&blank_ext, "voice_transcribe_bytes blank ext").contains("must not be empty"),
        "an extension that trims to nothing must be refused"
    );

    let missing_bytes = harness
        .rpc(85_008, "openhuman.voice_transcribe_bytes", json!({}))
        .await;
    assert!(
        err_message(&missing_bytes, "voice_transcribe_bytes missing")
            .contains("missing required param 'audio_bytes'"),
        "an omitted `audio_bytes` must be refused at the dispatch boundary"
    );

    // voice_transcribe reads a file path; a path that does not exist must fail
    // with a reason rather than returning an empty transcript, which the UI
    // would render as "no speech detected".
    let missing_file = harness
        .rpc(
            85_009,
            "openhuman.voice_transcribe",
            json!({ "audio_path": harness.home().join("nope.wav").to_str().unwrap(), "skip_cleanup": true }),
        )
        .await;
    assert!(
        !err_message(&missing_file, "voice_transcribe missing file").is_empty(),
        "transcribing a nonexistent file must fail with a reason"
    );

    let no_path = harness
        .rpc(85_010, "openhuman.voice_transcribe", json!({}))
        .await;
    assert!(
        err_message(&no_path, "voice_transcribe no path")
            .contains("missing required param 'audio_path'"),
        "an omitted `audio_path` must be refused at the dispatch boundary"
    );

    harness.shutdown();
}

// ── voice: realtime agent signed URL ───────────────────────────────────────

#[tokio::test]
async fn voice_agent_signed_url_mints_from_the_backend_and_refuses_without_a_session() {
    let _lock = env_lock();
    let harness = setup("").await;

    let signed_out = harness
        .rpc(86_001, "openhuman.voice_agent_signed_url", json!({}))
        .await;
    assert!(
        err_message(&signed_out, "voice_agent_signed_url signed out")
            .contains("no backend session token; sign in first"),
        "minting without a session must say to sign in"
    );

    harness.store_session(86_002).await;

    let minted = harness
        .rpc(86_003, "openhuman.voice_agent_signed_url", json!({}))
        .await;
    let minted = payload(&minted, "voice_agent_signed_url");
    assert_eq!(
        minted.get("agent_id").or_else(|| minted.get("agentId")),
        Some(&json!("agent_e2e_001")),
        "the backend's agent id must be unwrapped from its envelope: {minted}"
    );
    let url = minted
        .get("signed_url")
        .or_else(|| minted.get("signedUrl"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(
        url, "wss://voice.e2e.test/convai?token=e2e-signed",
        "the signed URL must round-trip verbatim: {minted}"
    );

    harness.shutdown();
}

// ── voice: local Piper TTS ─────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn voice_tts_writes_a_local_wav_through_piper_and_reports_the_disabled_runtime() {
    let _lock = env_lock();

    // `runtime_enabled = false` is the first gate `local_ai::tts` checks, and
    // it is what a user with local AI switched off will hit.
    let disabled = setup("").await;
    let refused = disabled
        .rpc(87_001, "openhuman.voice_tts", json!({ "text": "hello" }))
        .await;
    assert!(
        err_message(&refused, "voice_tts disabled").contains("local ai is disabled"),
        "local TTS with the runtime off must say so"
    );
    let no_text = disabled.rpc(87_002, "openhuman.voice_tts", json!({})).await;
    assert!(
        err_message(&no_text, "voice_tts no text").contains("missing required param 'text'"),
        "an omitted `text` must be refused at the dispatch boundary"
    );
    disabled.shutdown();

    // Now the happy path, with the runtime on and a stub piper + stub voice.
    let harness = setup("[local_ai]\nruntime_enabled = true\n").await;
    let voice_model = harness.home().join("stub-voice.onnx");
    std::fs::write(&voice_model, b"not a real onnx, only a path").expect("write stub voice");
    let piper = write_piper_stub(&harness.home().join("stub-bin"));

    // `tts_voice_id` is read as a path first, so pointing it at a real file is
    // what lets `resolve_tts_voice_path` succeed without a model download.
    let _ = harness
        .rpc(
            87_003,
            "openhuman.voice_set_providers",
            json!({ "tts_voice": voice_model.to_str().unwrap() }),
        )
        .await;
    let _piper_guard = EnvVarGuard::set_to_path("PIPER_BIN", &piper);

    let out_path = harness.workspace().join("tts-e2e.wav");
    let synthesized = harness
        .rpc(
            87_004,
            "openhuman.voice_tts",
            json!({ "text": "hello local", "output_path": out_path.to_str().unwrap() }),
        )
        .await;
    let synthesized = payload(&synthesized, "voice_tts piper");
    assert_eq!(
        synthesized.get("output_path").and_then(Value::as_str),
        Some(out_path.display().to_string().as_str()),
        "the requested output path must be honoured and reported: {synthesized}"
    );
    let written = std::fs::read(&out_path).expect("piper must have written the wav");
    assert!(
        written.starts_with(b"RIFF"),
        "the reported file must actually be the synthesized audio, not an empty stub"
    );

    harness.shutdown();
}

// ── audio_toolkit ──────────────────────────────────────────────────────────

#[tokio::test]
async fn audio_toolkit_generates_emails_and_combines_the_podcast_flow() {
    let _lock = env_lock();
    let harness = setup("").await;
    harness.store_session(88_001).await;

    // Capture mode writes the composed MIME message to the workspace instead of
    // opening an SMTP connection, which is what makes the email half of this
    // namespace testable at all.
    let capture_dir = harness.workspace().join("email-capture");
    let _capture = EnvVarGuard::set_to_path("OPENHUMAN_EMAIL_CAPTURE_DIR", &capture_dir);

    // generate_podcast — cloud TTS via the mock backend, written under the
    // workspace.
    let generated = harness
        .rpc(
            88_002,
            "openhuman.audio_toolkit_generate_podcast",
            json!({
                "text": "Chapter one. The lazy senior developer.",
                "title": "Ponytail Weekly",
                "provider": "cloud",
                "format": "mp3"
            }),
        )
        .await;
    let generated = payload(&generated, "audio_toolkit_generate_podcast");
    assert_eq!(generated.get("provider"), Some(&json!("cloud")));
    assert_eq!(generated.get("format"), Some(&json!("mp3")));
    assert_eq!(generated.get("audio_mime"), Some(&json!("audio/mpeg")));
    assert_eq!(
        generated.get("chars_synthesized"),
        Some(&json!(39)),
        "the synthesized character count must reflect the trimmed input: {generated}"
    );
    let rel_path = generated
        .get("output_path")
        .and_then(Value::as_str)
        .expect("output_path")
        .to_string();
    assert!(
        rel_path.starts_with("artifacts/audio/"),
        "the default output path must land in the audio artifacts dir: {rel_path}"
    );
    assert!(
        rel_path.ends_with("-ponytail-weekly.mp3"),
        "the title must be slugified into the file name: {rel_path}"
    );
    let on_disk = harness.workspace().join(&rel_path);
    assert!(
        on_disk.is_file(),
        "the audio must actually be written to {}",
        on_disk.display()
    );
    assert_eq!(
        generated.get("bytes_written").and_then(Value::as_u64),
        Some(std::fs::metadata(&on_disk).expect("stat audio").len()),
        "the reported byte count must match the file on disk: {generated}"
    );

    // email_podcast — attach the file just generated.
    let emailed = harness
        .rpc(
            88_003,
            "openhuman.audio_toolkit_email_podcast",
            json!({
                "to": "listener@example.test",
                "subject": "Your podcast",
                "body": "Attached.",
                "audio_path": rel_path,
                "attachment_name": "weekly.mp3"
            }),
        )
        .await;
    let emailed = payload(&emailed, "audio_toolkit_email_podcast");
    assert_eq!(emailed.get("to"), Some(&json!("listener@example.test")));
    assert_eq!(emailed.get("subject"), Some(&json!("Your podcast")));
    assert_eq!(emailed.get("attachment_name"), Some(&json!("weekly.mp3")));
    assert_eq!(
        emailed.get("mode"),
        Some(&json!("capture")),
        "with a capture dir set the message must be captured, never sent: {emailed}"
    );
    let capture_rel = emailed
        .get("capture_path")
        .and_then(Value::as_str)
        .expect("capture_path")
        .to_string();
    let eml = std::fs::read_to_string(harness.workspace().join(&capture_rel))
        .expect("the captured .eml must exist");
    assert!(
        eml.contains("To: listener@example.test"),
        "the captured message must address the recipient: {eml:.400}"
    );
    assert!(
        eml.contains("Subject: Your podcast"),
        "the captured message must carry the subject: {eml:.400}"
    );
    assert!(
        eml.contains("weekly.mp3"),
        "the attachment name must appear in the MIME part: {eml:.400}"
    );
    assert!(
        eml.contains("audio/mpeg"),
        "the attachment content type must be derived from the file name: {eml:.400}"
    );

    // generate_and_email_podcast — the combined flow must thread the generated
    // path into the email, which is the one thing the two-call version cannot
    // get wrong and the combined one can.
    let combined = harness
        .rpc(
            88_004,
            "openhuman.audio_toolkit_generate_and_email_podcast",
            json!({
                "text": "Combined flow.",
                "to": "listener@example.test",
                "subject": "Combined",
                "body": "Generated and sent.",
                "title": "Combined Episode",
                "provider": "cloud",
                "format": "mp3"
            }),
        )
        .await;
    let combined = payload(&combined, "audio_toolkit_generate_and_email_podcast");
    let audio_path = combined
        .pointer("/audio/output_path")
        .and_then(Value::as_str)
        .expect("combined audio output_path")
        .to_string();
    assert!(
        audio_path.ends_with("-combined-episode.mp3"),
        "the combined flow must slugify its own title: {audio_path}"
    );
    assert!(
        harness.workspace().join(&audio_path).is_file(),
        "the combined flow must write the audio it claims to have emailed"
    );
    assert_eq!(
        combined.pointer("/email/mode"),
        Some(&json!("capture")),
        "the combined flow must go through the same delivery path: {combined}"
    );
    let combined_attachment = combined
        .pointer("/email/attachment_name")
        .and_then(Value::as_str)
        .expect("combined attachment_name");
    assert!(
        audio_path.ends_with(combined_attachment),
        "with no explicit attachment name the generated file's name must be used \
         — this is the seam where an empty audio_path used to slip through: \
         audio={audio_path} attachment={combined_attachment}"
    );

    harness.shutdown();
}

#[tokio::test]
async fn audio_toolkit_refuses_bad_paths_formats_and_missing_fields() {
    let _lock = env_lock();
    let harness = setup("").await;
    let capture_dir = harness.workspace().join("email-capture");
    let _capture = EnvVarGuard::set_to_path("OPENHUMAN_EMAIL_CAPTURE_DIR", &capture_dir);

    // Provider/format incompatibilities are refused before any synthesis is
    // attempted — the user gets told which combination to pick.
    let cloud_wav = harness
        .rpc(
            89_001,
            "openhuman.audio_toolkit_generate_podcast",
            json!({ "text": "x", "provider": "cloud", "format": "wav" }),
        )
        .await;
    assert!(
        err_message(&cloud_wav, "generate_podcast cloud+wav")
            .contains("provider `cloud` currently returns mp3 output only"),
        "cloud+wav must be refused with the fix named"
    );

    let piper_mp3 = harness
        .rpc(
            89_002,
            "openhuman.audio_toolkit_generate_podcast",
            json!({ "text": "x", "provider": "piper", "format": "mp3" }),
        )
        .await;
    assert!(
        err_message(&piper_mp3, "generate_podcast piper+mp3")
            .contains("provider `piper` only supports wav output"),
        "piper+mp3 must be refused with the fix named"
    );

    // Empty text short-circuits before the provider is even constructed.
    let empty = harness
        .rpc(
            89_003,
            "openhuman.audio_toolkit_generate_podcast",
            json!({ "text": "   " }),
        )
        .await;
    assert!(
        err_message(&empty, "generate_podcast empty").contains("text is required"),
        "empty text must be refused"
    );

    // The workspace confinement on the output path is the security boundary of
    // this namespace: without it a caller could write anywhere on the host.
    let absolute = harness
        .rpc(
            89_004,
            "openhuman.audio_toolkit_generate_podcast",
            json!({ "text": "x", "provider": "cloud", "output_path": "/tmp/escape.mp3" }),
        )
        .await;
    assert!(
        err_message(&absolute, "generate_podcast absolute path")
            .contains("output_path must be workspace-relative"),
        "an absolute output path must be refused"
    );

    let traversal = harness
        .rpc(
            89_005,
            "openhuman.audio_toolkit_generate_podcast",
            json!({ "text": "x", "provider": "cloud", "output_path": "../../escape.mp3" }),
        )
        .await;
    assert!(
        err_message(&traversal, "generate_podcast traversal")
            .contains("must not contain parent-directory traversal"),
        "a traversing output path must be refused"
    );

    // The same confinement on the *read* side of email_podcast.
    for (id, path, expected) in [
        (
            89_010,
            "/etc/passwd",
            "audio_path must be workspace-relative",
        ),
        (
            89_011,
            "../../../etc/passwd",
            "must not contain parent-directory traversal",
        ),
    ] {
        let refused = harness
            .rpc(
                id,
                "openhuman.audio_toolkit_email_podcast",
                json!({
                    "to": "a@example.test",
                    "subject": "s",
                    "body": "b",
                    "audio_path": path
                }),
            )
            .await;
        assert!(
            err_message(&refused, "email_podcast confinement").contains(expected),
            "email_podcast must refuse '{path}'"
        );
    }

    // Each required email field is validated by name.
    for (id, field, params) in [
        (
            89_020,
            "to",
            json!({ "to": "  ", "subject": "s", "body": "b", "audio_path": "a.mp3" }),
        ),
        (
            89_021,
            "subject",
            json!({ "to": "a@example.test", "subject": " ", "body": "b", "audio_path": "a.mp3" }),
        ),
        (
            89_022,
            "body",
            json!({ "to": "a@example.test", "subject": "s", "body": " ", "audio_path": "a.mp3" }),
        ),
        (
            89_023,
            "audio_path",
            json!({ "to": "a@example.test", "subject": "s", "body": "b", "audio_path": " " }),
        ),
    ] {
        let refused = harness
            .rpc(id, "openhuman.audio_toolkit_email_podcast", params)
            .await;
        let message = err_message(&refused, &format!("email_podcast blank {field}"));
        assert!(
            message.contains(&format!("{field} is required")),
            "a blank `{field}` must be named in the error, got: {message}"
        );
    }

    // A workspace-relative path that simply is not there.
    let absent = harness
        .rpc(
            89_030,
            "openhuman.audio_toolkit_email_podcast",
            json!({
                "to": "a@example.test",
                "subject": "s",
                "body": "b",
                "audio_path": "artifacts/audio/nope.mp3"
            }),
        )
        .await;
    assert!(
        err_message(&absent, "email_podcast absent file").contains("failed to read"),
        "emailing a missing attachment must fail with a read error"
    );

    // The combined flow inherits every one of those guards.
    let combined_bad = harness
        .rpc(
            89_040,
            "openhuman.audio_toolkit_generate_and_email_podcast",
            json!({
                "text": "x",
                "to": "a@example.test",
                "subject": "s",
                "body": "b",
                "provider": "cloud",
                "format": "wav"
            }),
        )
        .await;
    assert!(
        err_message(&combined_bad, "generate_and_email cloud+wav")
            .contains("provider `cloud` currently returns mp3 output only"),
        "the combined flow must validate before it generates"
    );

    // …and must not have written anything on the way to that refusal.
    assert!(
        !harness.workspace().join("artifacts").exists(),
        "a refused generate must leave no artifacts directory behind"
    );

    harness.shutdown();
}
