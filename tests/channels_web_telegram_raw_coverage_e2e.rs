//! Round18 raw integration coverage for web-channel and Telegram provider paths.
//!
//! The tests use loopback mocks and existing debug seams only. No real channel
//! credentials, provider tokens, or external services are required.

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
use openhuman_core::core::event_bus::{init_global, publish_global, DomainEvent};
use openhuman_core::openhuman::channels::providers::telegram::TelegramChannel;
use openhuman_core::openhuman::channels::providers::web::{
    cancel_chat, register_approval_surface_subscriber, start_chat, subscribe_web_channel_events,
    test_support as web_test_support,
};
use openhuman_core::openhuman::channels::providers::yuanbao::{YuanbaoChannel, YuanbaoConfig};
use openhuman_core::openhuman::channels::LarkChannel;
use openhuman_core::openhuman::channels::{Channel, SendMessage};
use openhuman_core::openhuman::config::{schema::LarkConfig, StreamMode};
use serde_json::{json, Value};
use tokio::time::timeout;

#[derive(Debug, Clone)]
struct RecordedTelegramRequest {
    method: String,
    headers: HeaderMap,
    body: Value,
}

#[derive(Default)]
struct TelegramMockState {
    requests: Mutex<Vec<RecordedTelegramRequest>>,
    update_calls: Mutex<u32>,
}

async fn telegram_mock_handler(
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
        .expect("telegram requests lock")
        .push(RecordedTelegramRequest {
            method: method.clone(),
            headers,
            body: parsed.clone(),
        });

    match method.as_str() {
        "getMe" => {
            return (
                StatusCode::OK,
                axum::Json(json!({
                    "ok": true,
                    "result": { "id": 1, "username": "OpenHumanBot" },
                })),
            );
        }
        "getUpdates" => {
            let mut calls = state.update_calls.lock().expect("update calls lock");
            *calls += 1;
            let payload = match *calls {
                1 => json!({
                    "ok": false,
                    "error_code": 409,
                    "description": "Conflict: terminated by other getUpdates request; webhook is active",
                }),
                2 => json!({
                    "ok": true,
                    "result": [
                        {
                            "update_id": 100,
                            "message": {
                                "message_id": 501,
                                "message_thread_id": 77,
                                "text": "@OpenHumanBot please cover inbound parsing",
                                "from": { "id": 77, "username": "allowed_user" },
                                "chat": { "id": -1001, "type": "supergroup" },
                                "reply_to_message": { "message_id": 490 }
                            }
                        },
                        {
                            "update_id": 100,
                            "message": {
                                "message_id": 502,
                                "text": "@OpenHumanBot duplicate should be skipped",
                                "from": { "id": 77, "username": "allowed_user" },
                                "chat": { "id": -1001, "type": "supergroup" }
                            }
                        },
                        {
                            "update_id": 101,
                            "message_reaction": {
                                "chat": { "id": -1001 },
                                "message_id": 501,
                                "user": { "id": 77, "username": "allowed_user" },
                                "new_reaction": [{ "type": "emoji", "emoji": "👍" }]
                            }
                        },
                        {
                            "update_id": 102,
                            "message": {
                                "message_id": 503,
                                "text": "unauthorized should trigger approval prompt",
                                "from": { "id": 88, "username": "blocked_user" },
                                "chat": { "id": -1001, "type": "private" }
                            }
                        }
                    ]
                }),
                _ => json!({ "ok": true, "result": [] }),
            };
            return (StatusCode::OK, axum::Json(payload));
        }
        "deleteWebhook" => {
            return (
                StatusCode::OK,
                axum::Json(json!({ "ok": true, "result": true })),
            );
        }
        "setMessageReaction" => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "ok": false,
                    "description": "reaction unavailable in this chat",
                })),
            );
        }
        "sendMessage" => {
            let markdown = parsed.get("parse_mode").and_then(Value::as_str) == Some("Markdown");
            let text = parsed
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if markdown && text.contains("markdown-fallback") {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "ok": false,
                        "description": "mock markdown parse failure",
                    })),
                );
            }
            if text.contains("plain-fails-too") {
                return (
                    StatusCode::BAD_GATEWAY,
                    axum::Json(json!({
                        "ok": false,
                        "description": "mock plain send failure",
                    })),
                );
            }
            return (
                StatusCode::OK,
                axum::Json(json!({
                    "ok": true,
                    "result": { "message_id": 9101 },
                })),
            );
        }
        "sendVideo" | "sendAudio" | "sendVoice" | "sendDocument" | "sendPhoto"
        | "sendChatAction" | "editMessageText" | "deleteMessage" => {
            return (
                StatusCode::OK,
                axum::Json(json!({ "ok": true, "result": true })),
            );
        }
        _ => {}
    }

    (
        StatusCode::OK,
        axum::Json(json!({
            "ok": true,
            "result": true,
        })),
    )
}

async fn spawn_telegram_mock() -> (String, Arc<TelegramMockState>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(TelegramMockState::default());
    let app = Router::new()
        .route(
            "/bot{token}/{method}",
            post(telegram_mock_handler).get(telegram_mock_handler),
        )
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind telegram mock");
    let addr = listener.local_addr().expect("mock local addr");
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
        // SAFETY: this integration test binary mutates only its own process env
        // before constructing Telegram clients that read these variables.
        unsafe {
            std::env::set_var(key, value.as_ref());
        }
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: restores the process env slot changed by EnvGuard::set.
        unsafe {
            match self.old.as_deref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[tokio::test]
async fn web_channel_approval_bridge_forced_errors_and_newer_request_cancellation() {
    init_global(64);
    register_approval_surface_subscriber();
    let mut rx = subscribe_web_channel_events();

    publish_global(DomainEvent::ApprovalRequested {
        request_id: "round18-approval".to_string(),
        tool_name: "filesystem.write".to_string(),
        action_summary: "write a test artifact".to_string(),
        args_redacted: json!({ "path": "target/channels-web-telegram-round18-artifact" }),
        thread_id: Some("round18-thread".to_string()),
        client_id: Some("round18-client".to_string()),
    });

    let approval = timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("web channel event");
            if event.request_id == "round18-approval" {
                break event;
            }
        }
    })
    .await
    .expect("approval event");
    assert_eq!(approval.event, "approval_request");
    assert_eq!(approval.tool_name.as_deref(), Some("filesystem.write"));
    assert!(approval
        .message
        .as_deref()
        .expect("approval message")
        .contains("write a test artifact"));
    assert_eq!(
        approval.args.as_ref().and_then(|args| args.get("path")),
        Some(&json!("target/channels-web-telegram-round18-artifact"))
    );

    publish_global(DomainEvent::ApprovalRequested {
        request_id: "round18-approval-without-chat".to_string(),
        tool_name: "filesystem.write".to_string(),
        action_summary: "missing chat routing".to_string(),
        args_redacted: json!({}),
        thread_id: None,
        client_id: Some("round18-client".to_string()),
    });

    web_test_support::set_forced_run_chat_task_error_for_test(Some(
        "All providers/models failed. Attempts: openai API error (503 Service Unavailable)",
    ))
    .await;
    let forced_id = start_chat(
        "round18-client",
        "round18-forced-error",
        "trigger the forced provider error",
        Some("gpt-test".to_string()),
        Some(0.3),
        Some("missing-profile".to_string()),
        Some("en-US".to_string()),
        None,
    )
    .await
    .expect("forced chat accepted");
    let forced_error = timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("web channel event");
            if event.request_id == forced_id && event.event == "chat_error" {
                break event;
            }
        }
    })
    .await
    .expect("forced error event");
    assert_eq!(forced_error.error_type.as_deref(), Some("provider_error"));
    assert_eq!(forced_error.error_fallback_available, Some(false));

    let first_id = start_chat(
        "round18-client",
        "round18-shared-thread",
        "first request should be superseded",
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("first chat accepted");
    let second_id = start_chat(
        "round18-client-reconnect",
        "round18-shared-thread",
        "second request cancels the first",
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("second chat accepted");
    assert_ne!(first_id, second_id);

    let superseded = timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("web channel event");
            if event.request_id == first_id && event.event == "chat_error" {
                break event;
            }
        }
    })
    .await
    .expect("superseded cancellation event");
    assert_eq!(
        superseded.message.as_deref(),
        Some("Cancelled by newer request")
    );
    assert_eq!(superseded.error_type.as_deref(), Some("cancelled"));

    assert_eq!(
        cancel_chat("round18-client-reconnect", "round18-shared-thread")
            .await
            .expect("cancel second"),
        Some(second_id)
    );

    // Clear the forced error so subsequent tests in this binary are not affected.
    web_test_support::set_forced_run_chat_task_error_for_test(None).await;
}

#[tokio::test]
async fn telegram_loopback_covers_polling_recovery_inbound_reaction_and_send_errors() {
    let (base, state, server) = spawn_telegram_mock().await;
    let _api_base = EnvGuard::set("OPENHUMAN_TELEGRAM_BOT_API_BASE", &base);
    let _legacy_base = EnvGuard::set("OPENHUMAN_TELEGRAM_API_BASE", "");

    let channel = TelegramChannel::new(
        "ROUND18_TOKEN".to_string(),
        vec!["allowed_user".to_string(), "77".to_string()],
        true,
    )
    .with_streaming(StreamMode::Partial, 0, false);

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let listen_handle = tokio::spawn(async move { channel.listen(tx).await });
    let inbound = timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("inbound timeout")
        .expect("inbound message");
    assert_eq!(inbound.channel, "telegram");
    assert_eq!(inbound.sender, "allowed_user");
    assert_eq!(inbound.reply_target, "-1001:77");
    assert_eq!(inbound.thread_ts.as_deref(), Some("501"));
    assert_eq!(inbound.content, "please cover inbound parsing");
    listen_handle.abort();

    let channel = TelegramChannel::new("ROUND18_TOKEN".to_string(), vec!["*".to_string()], false)
        .with_streaming(StreamMode::Partial, 0, false);
    channel
        .send(&SendMessage::new(
            "[REACTION:👍|44] markdown-fallback body",
            "123:77",
        ))
        .await
        .expect("reaction failure should not block text fallback");
    channel
        .send_video_by_url(
            "123",
            Some("77"),
            "https://example.test/video.mp4",
            Some("clip"),
        )
        .await
        .expect("video url send");
    channel
        .send_audio_by_url(
            "123",
            Some("77"),
            "https://example.test/audio.mp3",
            Some("audio"),
        )
        .await
        .expect("audio url send");
    channel
        .send_voice_by_url(
            "123",
            Some("77"),
            "https://example.test/voice.ogg",
            Some("voice"),
        )
        .await
        .expect("voice url send");
    channel
        .send(&SendMessage::new(
            "caption before markers [DOCUMENT:https://example.test/a.pdf][PHOTO:https://example.test/a.png]",
            "123:77",
        ))
        .await
        .expect("attachment marker url sends");
    let send_error = channel
        .send(&SendMessage::new("plain-fails-too", "123"))
        .await
        .expect_err("plain retry should fail");
    assert!(send_error
        .to_string()
        .contains("Telegram sendMessage failed"));
    assert!(channel.health_check().await);

    let requests = state
        .requests
        .lock()
        .expect("telegram requests lock")
        .clone();
    server.abort();

    assert!(requests
        .iter()
        .any(|req| req.method == "getMe" && req.headers.get("host").is_some()));
    assert!(requests.iter().any(|req| req.method == "getUpdates"));
    assert!(requests.iter().any(|req| req.method == "deleteWebhook"
        && req
            .body
            .get("drop_pending_updates")
            .and_then(Value::as_bool)
            == Some(false)));
    assert!(requests
        .iter()
        .any(|req| req.method == "setMessageReaction"));
    assert!(requests.iter().any(|req| req.method == "sendMessage"
        && req.body.get("parse_mode").and_then(Value::as_str) == Some("Markdown")));
    assert!(requests
        .iter()
        .any(|req| req.method == "sendMessage" && req.body.get("parse_mode").is_none()));
    assert!(requests.iter().any(|req| req.method == "sendVideo"
        && req.body.get("video").and_then(Value::as_str)
            == Some("https://example.test/video.mp4")));
    assert!(requests.iter().any(|req| req.method == "sendAudio"
        && req.body.get("audio").and_then(Value::as_str)
            == Some("https://example.test/audio.mp3")));
    assert!(requests.iter().any(|req| req.method == "sendVoice"
        && req.body.get("voice").and_then(Value::as_str)
            == Some("https://example.test/voice.ogg")));
    assert!(requests.iter().any(|req| req.method == "sendDocument"
        && req.body.get("document").and_then(Value::as_str) == Some("https://example.test/a.pdf")));
    assert!(requests.iter().any(|req| req.method == "sendPhoto"
        && req.body.get("photo").and_then(Value::as_str) == Some("https://example.test/a.png")));
}

#[test]
fn lark_and_yuanbao_accessible_config_and_parser_branches() {
    let mut lark_cfg = LarkConfig {
        app_id: "round18-app".to_string(),
        app_secret: "round18-secret".to_string(),
        encrypt_key: None,
        verification_token: Some("round18-verify".to_string()),
        port: Some(0),
        allowed_users: vec!["ou_allowed".to_string()],
        use_feishu: false,
        receive_mode: Default::default(),
    };
    let lark = LarkChannel::from_config(&lark_cfg);
    assert_eq!(lark.name(), "lark");

    let missing_event = json!({ "header": { "event_type": "url_verification" } });
    assert!(lark.parse_event_payload(&missing_event).is_empty());

    let empty_sender = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": {} },
            "message": { "message_type": "text", "content": "{\"text\":\"missing sender\"}" }
        }
    });
    assert!(lark.parse_event_payload(&empty_sender).is_empty());

    let malformed_text = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_allowed" } },
            "message": { "message_type": "text", "content": "{\"bad\":\"shape\"}" }
        }
    });
    assert!(lark.parse_event_payload(&malformed_text).is_empty());

    let unsupported_type = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_allowed" } },
            "message": { "message_type": "image", "content": "{}" }
        }
    });
    assert!(lark.parse_event_payload(&unsupported_type).is_empty());

    let text_payload = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_allowed" } },
            "message": {
                "message_type": "text",
                "content": "{\"text\":\"hello from round18\"}",
                "create_time": "1710000000123",
                "chat_id": "oc_chat"
            }
        }
    });
    let messages = lark.parse_event_payload(&text_payload);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].channel, "lark");
    assert_eq!(messages[0].sender, "oc_chat");
    assert_eq!(messages[0].content, "hello from round18");

    let post_payload = json!({
        "header": { "event_type": "im.message.receive_v1" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_allowed" } },
            "message": {
                "message_type": "post",
                "content": serde_json::to_string(&json!({
                    "en_us": {
                        "title": "Round18",
                        "content": [[
                            { "tag": "text", "text": "notes " },
                            { "tag": "a", "text": "link", "href": "https://example.test" },
                            { "tag": "at", "user_name": "Ada" }
                        ]]
                    }
                })).expect("post json"),
                "chat_id": "oc_chat"
            }
        }
    });
    let post_messages = lark.parse_event_payload(&post_payload);
    assert_eq!(post_messages.len(), 1);
    assert!(post_messages[0].content.contains("Round18"));
    assert!(post_messages[0].content.contains("notes link@Ada"));

    lark_cfg.allowed_users = vec!["*".to_string()];
    let wildcard_lark = LarkChannel::from_config(&lark_cfg);
    assert_eq!(wildcard_lark.parse_event_payload(&text_payload).len(), 1);

    let mut prod = YuanbaoConfig {
        app_key: "ak".to_string(),
        token: "tok".to_string(),
        ..Default::default()
    };
    prod.apply_env_defaults();
    assert!(prod.api_domain.contains("bot.yuanbao.tencent.com"));
    assert!(prod.ws_domain.contains("bot-wss.yuanbao.tencent.com"));
    prod.validate().expect("prod token config validates");

    let channel = YuanbaoChannel::new(prod.clone()).expect("yuanbao channel");
    assert_eq!(channel.name(), "yuanbao");
    assert!(!channel.supports_reactions());
    assert!(channel.supports_draft_updates());

    let mut pre = YuanbaoConfig {
        env: "pre".to_string(),
        app_key: "ak".to_string(),
        token: "tok".to_string(),
        ..Default::default()
    };
    pre.apply_env_defaults();
    assert!(pre.api_domain.contains("bot-pre.yuanbao.tencent.com"));
    assert!(pre.ws_domain.contains("bot-wss-pre.yuanbao.tencent.com"));
    pre.validate().expect("pre token config validates");

    let mut explicit = YuanbaoConfig {
        env: "pre".to_string(),
        app_key: "ak".to_string(),
        token: "tok".to_string(),
        api_domain: "https://custom-api.example.test".to_string(),
        ws_domain: "wss://custom-ws.example.test".to_string(),
        ..Default::default()
    };
    explicit.apply_env_defaults();
    assert_eq!(explicit.api_domain, "https://custom-api.example.test");
    assert_eq!(explicit.ws_domain, "wss://custom-ws.example.test");

    let mut bad = YuanbaoConfig {
        ws_domain: "wss://example.test".to_string(),
        token: "tok".to_string(),
        ..Default::default()
    };
    assert!(bad.validate().is_err());
    bad.app_key = "ak".to_string();
    bad.token.clear();
    assert!(bad.validate().is_err());
    bad.app_secret = "secret".to_string();
    bad.api_domain = "https://api.example.test".to_string();
    bad.validate()
        .expect("app_secret plus api_domain config validates");
}
