//! Round22 focused raw coverage for high-miss channel web/Yuanbao paths.
//!
//! All networked branches use loopback servers or in-memory debug seams.

use std::sync::{Arc, Mutex};

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use openhuman_core::openhuman::channels::providers::telegram::TelegramChannel;
use openhuman_core::openhuman::channels::providers::web::{
    cancel_chat, start_chat, subscribe_web_channel_events, test_support as web_test_support,
};
use openhuman_core::openhuman::channels::providers::yuanbao::{
    connection::test_support as yuanbao_connection_test_support,
    cos::{cos_sign, get_cos_credentials, upload_to_cos, CosCredentials, CosSignInput},
    YuanbaoConfig,
};
use openhuman_core::openhuman::channels::test_support::resolve_yuanbao_app_secret_for_test;
use openhuman_core::openhuman::channels::{Channel, SendMessage};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::AuthService;
use serde_json::{json, Value};
use tempfile::tempdir;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    headers: HeaderMap,
    body: Value,
}

#[derive(Default)]
struct TelegramMockState {
    requests: Mutex<Vec<RecordedRequest>>,
}

async fn telegram_handler(
    Path((_token, method)): Path<(String, String)>,
    State(state): State<Arc<TelegramMockState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| {
        json!({
            "raw": String::from_utf8_lossy(&body).to_string(),
        })
    });
    state
        .requests
        .lock()
        .expect("telegram requests")
        .push(RecordedRequest {
            method: method.clone(),
            headers,
            body: parsed.clone(),
        });

    match method.as_str() {
        "setMessageReaction" => (
            StatusCode::OK,
            axum::Json(json!({"ok": true, "result": true})),
        ),
        "sendAudio" => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"ok": false, "description": "audio url rejected"})),
        ),
        _ => (
            StatusCode::OK,
            axum::Json(json!({"ok": true, "result": {"message_id": 42}})),
        ),
    }
}

async fn spawn_telegram_mock() -> (String, Arc<TelegramMockState>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(TelegramMockState::default());
    let app = Router::new()
        .route("/bot{token}/{method}", post(telegram_handler))
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind telegram mock");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), state, handle)
}

#[derive(Default)]
struct CosMockState {
    requests: Mutex<Vec<RecordedRequest>>,
}

async fn cos_handler(
    State(state): State<Arc<CosMockState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!({}));
    state
        .requests
        .lock()
        .expect("cos requests")
        .push(RecordedRequest {
            method: "genUploadInfo".to_string(),
            headers,
            body: parsed,
        });

    (
        StatusCode::OK,
        axum::Json(json!({
            "code": 0,
            "data": {
                "bucketName": "round22-bucket",
                "region": "ap-shanghai",
                "location": "dir with spaces/file name.png",
                "encryptTmpSecretId": "AKID",
                "encryptTmpSecretKey": "SECRET",
                "encryptToken": "session-token",
                "startTime": 1700000000u64,
                "expiredTime": 1700003600u64,
                "resourceUrl": "https://cdn.example/round22.png"
            }
        })),
    )
}

async fn spawn_cos_mock() -> (String, Arc<CosMockState>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(CosMockState::default());
    let app = Router::new()
        .route(
            "/api/resource/genUploadInfo",
            post(cos_handler).put(cos_handler),
        )
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind cos mock");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), state, handle)
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<str>) -> Self {
        let old = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value.as_ref());
        }
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.old.as_deref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn isolated_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    (tmp, config)
}

#[tokio::test]
async fn web_start_chat_validation_forced_error_and_cancel_paths_are_structured() {
    assert_eq!(
        start_chat(" ", "thread", "hello", None, None, None, None, None)
            .await
            .unwrap_err(),
        "client_id is required"
    );
    assert_eq!(
        start_chat("client", " ", "hello", None, None, None, None, None)
            .await
            .unwrap_err(),
        "thread_id is required"
    );

    web_test_support::set_forced_run_chat_task_error_for_test(Some(
        "All providers/models failed. Attempts: openhuman API error (503 Service Unavailable)",
    ))
    .await;
    let mut rx = subscribe_web_channel_events();
    let request_id = start_chat(
        "round22-client",
        "round22-thread",
        "Please respond through the forced error seam.",
        Some(" ".to_string()),
        Some(0.4),
        None,
        None,
        None,
    )
    .await
    .expect("accepted");

    let event = timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("web event");
            if event.event == "chat_error" && event.request_id == request_id {
                break event;
            }
        }
    })
    .await
    .expect("chat_error");
    assert_eq!(event.error_type.as_deref(), Some("provider_error"));
    assert_eq!(event.error_fallback_available, Some(false));

    let cancelled = cancel_chat("round22-client", "round22-thread")
        .await
        .expect("cancel after completion is ok");
    assert_eq!(cancelled, None);
    web_test_support::set_forced_run_chat_task_error_for_test(None).await;
}

#[tokio::test]
async fn yuanbao_cos_credentials_signing_and_connection_debug_paths() {
    let (cos_base, cos_state, cos_server) = spawn_cos_mock().await;
    let http = reqwest::Client::new();
    let creds = get_cos_credentials(
        &http, &cos_base, "app-key", "", "token", "canary", "file.png",
    )
    .await
    .expect("credentials");
    assert_eq!(creds.bucket, "round22-bucket");
    assert_eq!(creds.location, "dir with spaces/file name.png");

    let requests = cos_state.requests.lock().expect("cos requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "genUploadInfo");
    assert_eq!(
        requests[0]
            .headers
            .get("x-id")
            .and_then(|h| h.to_str().ok()),
        Some("app-key")
    );
    assert_eq!(
        requests[0]
            .headers
            .get("x-route-env")
            .and_then(|h| h.to_str().ok()),
        Some("canary")
    );
    assert_eq!(requests[0].body["fileName"], "file.png");
    drop(requests);

    let signature = cos_sign(&CosSignInput {
        method: "PUT",
        path: "/dir%20with%20spaces/file%20name.png",
        params: &[("Param", "value with spaces")],
        headers: &[("Host", "round22-bucket.cos.accelerate.myqcloud.com")],
        secret_id: "AKID",
        secret_key: "SECRET",
        start_time: 1_700_000_000,
        expire_seconds: 300,
    });
    assert!(signature.contains("q-header-list=host"));
    assert!(signature.contains("q-url-param-list=param"));

    let err = upload_to_cos(
        &http,
        &CosCredentials::default(),
        b"not uploaded",
        "file.png",
        String::new(),
    )
    .await
    .unwrap_err();
    assert!(format!("{err:?}").contains("credentials missing"));

    assert_eq!(
        yuanbao_connection_test_support::auth_response_success_connect_id_for_test()
            .expect("auth response"),
        "connect-123"
    );
    assert!(
        yuanbao_connection_test_support::auth_response_rejects_status_for_test()
            .contains("status=401")
    );
    let events =
        yuanbao_connection_test_support::handle_binary_routes_builtin_and_push_frames_for_test()
            .await;
    assert_eq!(
        events,
        vec!["kickout:logged out", "push:incoming-message:push-1"]
    );

    cos_server.abort();
}

#[tokio::test]
async fn startup_yuanbao_secret_hydration_respects_matching_app_key() {
    let (_tmp, config) = isolated_config();
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        "channel:yuanbao:api_key",
        "default",
        "",
        [
            ("app_key".to_string(), "round22-key".to_string()),
            ("app_secret".to_string(), "round22-secret".to_string()),
        ]
        .into_iter()
        .collect(),
        true,
    )
    .expect("store credentials");

    let hydrated = resolve_yuanbao_app_secret_for_test(
        YuanbaoConfig {
            app_key: "round22-key".to_string(),
            app_secret: String::new(),
            ..Default::default()
        },
        &config,
    );
    assert_eq!(hydrated.app_secret, "round22-secret");

    let stale = resolve_yuanbao_app_secret_for_test(
        YuanbaoConfig {
            app_key: "other-key".to_string(),
            app_secret: String::new(),
            ..Default::default()
        },
        &config,
    );
    assert_eq!(stale.app_secret, "");
}

#[tokio::test]
async fn telegram_send_reaction_attachment_and_media_url_paths_use_loopback_api() {
    let (base, state, server) = spawn_telegram_mock().await;
    let _base_guard = EnvGuard::set("OPENHUMAN_TELEGRAM_BOT_API_BASE", base);
    let channel = TelegramChannel::new("TEST:TOKEN".to_string(), vec!["*".to_string()], false);

    channel
        .send(
            &SendMessage::new(
                "[REACTION:✅|321] Reply after reacting.\n[VIDEO:https://example.test/v.mp4]",
                "chat-1:99",
            )
            .in_thread(Some("123".to_string())),
        )
        .await
        .expect("reaction plus video url");

    let audio_err = channel
        .send(&SendMessage::new(
            "[AUDIO:https://example.test/a.mp3]",
            "chat-1",
        ))
        .await
        .unwrap_err();
    assert!(audio_err.to_string().contains("sendAudio by URL failed"));

    let requests = state.requests.lock().expect("telegram requests");
    assert!(requests.iter().any(|request| {
        request.method == "setMessageReaction"
            && request.body["message_id"] == 321
            && request.body["reaction"][0]["emoji"] == "✅"
    }));
    assert!(requests
        .iter()
        .any(|request| request.method == "sendMessage"
            && request.body["message_thread_id"] == "99"
            && request.body["reply_to_message_id"] == 123));
    assert!(requests.iter().any(|request| request.method == "sendVideo"
        && request.body["video"] == "https://example.test/v.mp4"));
    assert!(requests.iter().any(|request| request.method == "sendAudio"
        && request.body["audio"] == "https://example.test/a.mp3"));

    server.abort();
}
