//! Round19 focused raw coverage for leftover channel provider branches.
//!
//! These tests use loopback mocks, public debug seams, and short-lived
//! in-process listeners. They do not require real channel credentials or tokens.

use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    connection::YuanbaoConnection, YuanbaoChannel, YuanbaoConfig,
};
use openhuman_core::openhuman::channels::{Channel, LarkChannel, SendMessage};
use openhuman_core::openhuman::config::{schema::LarkConfig, StreamMode};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

#[derive(Debug, Clone)]
struct RecordedTelegramRequest {
    method: String,
    headers: HeaderMap,
    body: Value,
    raw_body: String,
}

#[derive(Default)]
struct TelegramMockState {
    requests: Mutex<Vec<RecordedTelegramRequest>>,
    updates_seen: Mutex<u32>,
}

async fn telegram_handler(
    Path((_token, method)): Path<(String, String)>,
    State(state): State<Arc<TelegramMockState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let raw_body = String::from_utf8_lossy(&body).to_string();
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!({}));
    state
        .requests
        .lock()
        .expect("telegram request lock")
        .push(RecordedTelegramRequest {
            method: method.clone(),
            headers,
            body: parsed.clone(),
            raw_body,
        });

    match method.as_str() {
        "getMe" => (
            StatusCode::OK,
            axum::Json(json!({
                "ok": true,
                "result": { "id": 19, "username": "Round19Bot" },
            })),
        ),
        "getUpdates" => {
            let mut seen = state.updates_seen.lock().expect("updates lock");
            *seen += 1;
            let payload = if *seen == 1 {
                json!({
                    "ok": true,
                    "result": [
                        {
                            "update_id": 20,
                            "message": {
                                "message_id": 300,
                                "text": "group message without mention is ignored",
                                "from": { "id": 88, "username": "allowed" },
                                "chat": { "id": -100, "type": "supergroup" }
                            }
                        },
                        {
                            "update_id": 21,
                            "edited_message": {
                                "message_id": 301,
                                "text": "hi @Round19Bot   normalize   this",
                                "from": { "id": 88, "username": "allowed" },
                                "chat": { "id": -100, "type": "supergroup" }
                            }
                        },
                        {
                            "update_id": 22,
                            "message_reaction": {
                                "chat": { "id": -100 },
                                "message_id": 301,
                                "user": { "id": 88 },
                                "new_reaction": [{ "type": "emoji", "emoji": "✅" }]
                            }
                        },
                        {
                            "update_id": 23,
                            "message": {
                                "message_id": 302,
                                "text": "/bind missing",
                                "from": { "id": 99, "username": "blocked" },
                                "chat": { "id": 555, "type": "private" }
                            }
                        }
                    ]
                })
            } else {
                json!({ "ok": true, "result": [] })
            };
            (StatusCode::OK, axum::Json(payload))
        }
        "sendChatAction" => {
            if parsed.get("message_thread_id").is_some() {
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({ "ok": false, "description": "topic action rejected" })),
                )
            } else {
                (
                    StatusCode::OK,
                    axum::Json(json!({ "ok": true, "result": true })),
                )
            }
        }
        "editMessageText" => {
            let markdown = parsed.get("parse_mode").and_then(Value::as_str) == Some("Markdown");
            if markdown {
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({ "ok": false, "description": "markdown edit rejected" })),
                )
            } else {
                (
                    StatusCode::OK,
                    axum::Json(json!({ "ok": true, "result": true })),
                )
            }
        }
        "deleteMessage" | "setMessageReaction" => (
            StatusCode::OK,
            axum::Json(json!({ "ok": true, "result": true })),
        ),
        "sendMessage" => (
            StatusCode::OK,
            axum::Json(json!({ "ok": true, "result": { "message_id": 777 } })),
        ),
        "sendDocument" | "sendPhoto" | "sendVideo" | "sendAudio" | "sendVoice" => (
            StatusCode::OK,
            axum::Json(json!({ "ok": true, "result": true })),
        ),
        _ => (
            StatusCode::OK,
            axum::Json(json!({ "ok": true, "result": true })),
        ),
    }
}

async fn spawn_telegram_mock() -> (String, Arc<TelegramMockState>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(TelegramMockState::default());
    let app = Router::new()
        .route(
            "/bot{token}/{method}",
            post(telegram_handler).get(telegram_handler),
        )
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind telegram mock");
    let addr = listener.local_addr().expect("telegram mock addr");
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

#[tokio::test]
async fn telegram_round19_covers_mention_filter_typing_fallback_and_attachment_forms() {
    let (base, state, server) = spawn_telegram_mock().await;
    let _api_base = EnvGuard::set("OPENHUMAN_TELEGRAM_BOT_API_BASE", &base);
    let _legacy_base = EnvGuard::set("OPENHUMAN_TELEGRAM_API_BASE", "");

    let listener_channel = TelegramChannel::new("ROUND19_TOKEN".into(), vec!["88".into()], true)
        .with_streaming(StreamMode::Partial, 0, false);
    let (tx, mut rx) = mpsc::channel(4);
    let listen_handle = tokio::spawn(async move { listener_channel.listen(tx).await });
    let inbound = timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("telegram inbound timeout")
        .expect("telegram inbound message");
    assert_eq!(inbound.content, "hi normalize this");
    assert_eq!(inbound.sender, "allowed");
    assert_eq!(inbound.thread_ts.as_deref(), Some("301"));
    listen_handle.abort();

    let channel = TelegramChannel::new("ROUND19_TOKEN".into(), vec!["*".into()], false)
        .with_streaming(StreamMode::Partial, 0, false);
    channel
        .start_typing("123:456")
        .await
        .expect("typing fallback");
    channel.stop_typing("123:456").await.expect("stop typing");

    let draft_id = channel
        .send_draft(&SendMessage::new("draft", "123:456").in_thread(Some("33".to_string())))
        .await
        .expect("draft send")
        .expect("draft message id");
    channel
        .finalize_draft(
            "123:456",
            &draft_id,
            "**markdown edit fallback**",
            Some("33"),
        )
        .await
        .expect("plain edit fallback");
    channel
        .update_draft("123:456", "not-an-int", "ignored invalid edit id")
        .await
        .expect("invalid edit id is ignored");

    let tmp = tempfile::tempdir().expect("tempdir");
    let doc = tmp.path().join("round19.txt");
    let photo = tmp.path().join("round19.jpg");
    tokio::fs::write(&doc, b"doc bytes")
        .await
        .expect("doc write");
    tokio::fs::write(&photo, b"photo bytes")
        .await
        .expect("photo write");
    channel
        .send_document("123", Some("456"), &doc, Some("doc caption"))
        .await
        .expect("document multipart");
    channel
        .send_photo("123", Some("456"), &photo, Some("photo caption"))
        .await
        .expect("photo multipart");
    let missing = channel
        .send(&SendMessage::new(
            format!("[DOCUMENT:{}]", tmp.path().join("missing.pdf").display()),
            "123:456",
        ))
        .await
        .expect_err("missing attachment path");
    assert!(missing.to_string().contains("path not found"));

    let requests = state
        .requests
        .lock()
        .expect("telegram requests lock")
        .clone();
    server.abort();

    assert!(requests.iter().any(|req| req.method == "getMe"));
    assert!(requests.iter().any(|req| req.method == "getUpdates"));
    assert!(requests
        .iter()
        .any(|req| req.method == "sendChatAction" && req.body.get("message_thread_id").is_some()));
    assert!(requests
        .iter()
        .any(|req| req.method == "sendChatAction" && req.body.get("message_thread_id").is_none()));
    assert!(requests.iter().any(|req| req.method == "editMessageText"
        && req.body.get("parse_mode").and_then(Value::as_str) == Some("Markdown")));
    assert!(requests
        .iter()
        .any(|req| req.method == "editMessageText" && req.body.get("parse_mode").is_none()));
    assert!(requests
        .iter()
        .any(|req| req.method == "sendDocument" && req.raw_body.contains("doc caption")));
    assert!(requests
        .iter()
        .any(|req| req.method == "sendPhoto" && req.raw_body.contains("photo caption")));
    assert!(requests.iter().any(|req| req.headers.get("host").is_some()));
}

#[tokio::test]
async fn web_round19_covers_classifier_variants_and_cancel_cleanup() {
    let auth = web_test_support::classify_error_for_test(
        "custom_openai API error (401 Unauthorized): invalid api key",
    );
    assert_eq!(auth.error_type, "auth_error");
    assert_eq!(auth.source, "config");
    assert!(!auth.retryable);

    // Issue #3088: budget-signal strings now classify as `budget_exhausted`
    // instead of falling through to the generic `inference` branch — the
    // user gets an actionable "top up or switch routing" message.
    let budget = web_test_support::classify_error_for_test(
        "inference budget exceeded: monthly limit reached",
    );
    assert_eq!(budget.error_type, "budget_exhausted");
    assert_eq!(budget.source, "openhuman_billing");

    let network = web_test_support::classify_error_for_test(
        "request error: dns error while trying to connect",
    );
    assert_eq!(network.error_type, "inference");
    assert!(network.retryable);

    web_test_support::set_forced_run_chat_task_error_for_test(Some(
        "Agent exceeded maximum tool iterations",
    ))
    .await;
    let mut rx = subscribe_web_channel_events();
    let request_id = start_chat(
        "round19-client",
        "round19-thread",
        "exercise deterministic max iteration classification",
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("start forced web chat");
    let event = timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("web channel event");
            if event.request_id == request_id && event.event == "chat_error" {
                break event;
            }
        }
    })
    .await
    .expect("forced chat_error");
    assert_eq!(event.error_type.as_deref(), Some("max_iterations"));
    web_test_support::set_forced_run_chat_task_error_for_test(None).await;

    assert_eq!(
        cancel_chat("round19-client", "round19-thread")
            .await
            .expect("cancel cleaned up forced request"),
        None
    );
}

#[test]
fn lark_round19_covers_parse_leftovers_and_config_defaults() {
    let mut cfg = LarkConfig {
        app_id: "round19-app".into(),
        app_secret: "round19-secret".into(),
        encrypt_key: None,
        verification_token: Some("round19-token".into()),
        port: Some(0),
        allowed_users: vec!["ou_allowed".into()],
        use_feishu: true,
        receive_mode: Default::default(),
    };
    let lark = LarkChannel::from_config(&cfg);
    assert_eq!(lark.name(), "lark");

    let fallback_locale_post = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_allowed" } },
            "message": {
                "message_type": "post",
                "content": serde_json::to_string(&json!({
                    "fr_fr": {
                        "content": [[
                            { "tag": "a", "href": "https://example.test/fallback" },
                            { "tag": "at", "user_id": "ou_friend" },
                            { "tag": "img", "image_key": "ignored" }
                        ]]
                    }
                })).expect("post json"),
                "chat_id": "oc_round19"
            }
        }
    });
    let messages = lark.parse_event_payload(&fallback_locale_post);
    assert_eq!(messages.len(), 1);
    assert!(messages[0]
        .content
        .contains("https://example.test/fallback"));
    assert!(messages[0].content.contains("@ou_friend"));

    let empty_post = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_allowed" } },
            "message": { "message_type": "post", "content": "{\"en_us\":{\"content\":[]}}" }
        }
    });
    assert!(lark.parse_event_payload(&empty_post).is_empty());

    let unsupported = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_allowed" } },
            "message": { "message_type": "image", "content": "{}" }
        }
    });
    assert!(lark.parse_event_payload(&unsupported).is_empty());

    cfg.use_feishu = false;
    cfg.allowed_users = vec!["*".into()];
    let wildcard = LarkChannel::from_config(&cfg);
    let mut text_payload = unsupported;
    text_payload["event"]["message"]["message_type"] = json!("text");
    text_payload["event"]["message"]["content"] = json!("{\"text\":\"wildcard accepted\"}");
    assert_eq!(wildcard.parse_event_payload(&text_payload).len(), 1);
}

#[tokio::test]
async fn lark_round19_listen_http_missing_port_and_ephemeral_bind_paths() {
    let missing_port = LarkChannel::from_config(&LarkConfig {
        app_id: "round19-app".into(),
        app_secret: "round19-secret".into(),
        encrypt_key: None,
        verification_token: Some("round19-token".into()),
        port: None,
        allowed_users: vec!["*".into()],
        use_feishu: true,
        receive_mode: Default::default(),
    });
    let (tx, _rx) = mpsc::channel(1);
    let err = missing_port
        .listen_http(tx)
        .await
        .expect_err("missing webhook port errors");
    assert!(err.to_string().contains("requires `port`"));

    let channel = LarkChannel::from_config(&LarkConfig {
        app_id: "round19-app".into(),
        app_secret: "round19-secret".into(),
        encrypt_key: None,
        verification_token: Some("round19-token".into()),
        port: Some(0),
        allowed_users: vec!["*".into()],
        use_feishu: true,
        receive_mode: Default::default(),
    });
    let (tx, _rx) = mpsc::channel(1);
    let handle = tokio::spawn(async move { channel.listen_http(tx).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!handle.is_finished());
    handle.abort();
}

#[tokio::test]
async fn yuanbao_round19_connection_run_shutdown_and_channel_error_paths() {
    let cfg = YuanbaoConfig {
        app_key: "round19-ak".into(),
        token: "round19-token".into(),
        bot_id: "round19-bot".into(),
        ws_domain: "ws://127.0.0.1:9/round19".into(),
        api_domain: "http://127.0.0.1:9".into(),
        heartbeat_interval_secs: 1,
        max_reconnect_attempts: 1,
        max_message_length: 12,
        ..Default::default()
    };
    cfg.validate().expect("valid static-token config");

    let (inbound_tx, _inbound_rx) = mpsc::unbounded_channel();
    let connection = YuanbaoConnection::new(cfg.clone(), inbound_tx, None);
    assert!(!connection.is_connected());
    assert_eq!(connection.account().uid, "round19-bot");
    let first = connection.next_msg_id("round19");
    let second = connection.next_msg_id("round19");
    assert_ne!(first, second);

    let send_err = connection
        .send_and_wait("missing", vec![1, 2, 3], Duration::from_millis(20))
        .await
        .expect_err("not connected send_and_wait");
    assert!(send_err.to_string().contains("not connected"));

    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    timeout(
        Duration::from_secs(5),
        Arc::clone(&connection).run(shutdown_rx),
    )
    .await
    .expect("connection run exits after retry budget");
    assert!(!connection.is_connected());
    connection.shutdown().await;

    let channel = YuanbaoChannel::new(cfg).expect("yuanbao channel");
    assert_eq!(channel.name(), "yuanbao");
    assert!(channel.supports_draft_updates());
    assert!(!channel.health_check().await);
    assert_eq!(
        channel
            .send_draft(&SendMessage::new("hello", "recipient"))
            .await
            .expect("yuanbao draft marker")
            .as_deref(),
        Some("yb-draft:recipient")
    );
    let err = channel
        .send(&SendMessage::new("split me into chunks please", "g:group"))
        .await
        .expect_err("not connected outbound send");
    assert!(err.to_string().contains("not connected"));
    assert!(channel
        .update_draft("recipient", "draft", "ignored")
        .await
        .is_ok());

    let mut bad = YuanbaoConfig {
        app_key: "round19-ak".into(),
        token: String::new(),
        app_secret: "round19-secret".into(),
        ws_domain: "wss://example.test/ws".into(),
        api_domain: String::new(),
        ..Default::default()
    };
    assert!(bad.validate().is_err());
    bad.api_domain = "https://api.example.test".into();
    bad.validate().expect("secret config needs api domain");
}

#[tokio::test]
async fn round19_artifact_target_prefix_is_used() {
    let dir = tempfile::Builder::new()
        .prefix("channels-provider-leftovers-round19-")
        .tempdir_in("target")
        .expect("round19 artifact dir");
    let marker = dir.path().join("marker.txt");
    tokio::fs::write(&marker, b"round19")
        .await
        .expect("marker write");
    assert!(marker
        .to_string_lossy()
        .contains("channels-provider-leftovers-round19-"));
}
