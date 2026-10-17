//! Round26 raw integration coverage for high-yield channel cold paths.
//!
//! Loopback Bot API endpoints only: no real channel
//! network services are contacted.
//!
//! Yuanbao biz-codec cases live in `vendor/tinychannels` (`yuanbao/proto_biz.rs`).

use crate::env_guard::EnvVarGuard;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use tinychannels::providers::telegram::TelegramChannel;
use tinychannels::{Channel, SendMessage};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct TelegramMockState {
    send_message_calls: Mutex<Vec<Value>>,
    reaction_calls: Mutex<Vec<Value>>,
    json_media_calls: Mutex<Vec<(String, Value)>>,
    multipart_calls: Mutex<Vec<(String, String)>>,
}

async fn spawn_telegram_mock() -> (String, Arc<TelegramMockState>) {
    let state = Arc::new(TelegramMockState::default());
    let app = Router::new()
        .route("/botround26/sendMessage", post(telegram_send_message))
        .route(
            "/botround26/setMessageReaction",
            post(telegram_set_reaction),
        )
        .route("/botround26/sendDocument", post(telegram_media))
        .route("/botround26/sendPhoto", post(telegram_media))
        .route("/botround26/sendVideo", post(telegram_media))
        .route("/botround26/sendAudio", post(telegram_media))
        .route("/botround26/sendVoice", post(telegram_media))
        .with_state(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind telegram mock");
    let addr = listener.local_addr().expect("telegram mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve telegram mock");
    });
    (format!("http://127.0.0.1:{}", addr.port()), state)
}

async fn telegram_send_message(
    State(state): State<Arc<TelegramMockState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut calls = state.send_message_calls.lock().expect("sendMessage calls");
    calls.push(body);
    if calls.len() == 1 {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "description": "markdown parse failed"})),
        )
    } else {
        (
            StatusCode::OK,
            Json(json!({"ok": true, "result": {"message_id": 7}})),
        )
    }
}

async fn telegram_set_reaction(
    State(state): State<Arc<TelegramMockState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let emoji = body
        .pointer("/reaction/0/emoji")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    state
        .reaction_calls
        .lock()
        .expect("reaction calls")
        .push(body);
    if emoji == "💥" {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "description": "reaction rejected"})),
        )
    } else {
        (StatusCode::OK, Json(json!({"ok": true})))
    }
}

async fn telegram_media(
    State(state): State<Arc<TelegramMockState>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let method = uri
        .path()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    if content_type.starts_with("application/json") {
        let value = serde_json::from_slice::<Value>(&body).expect("telegram media json");
        state
            .json_media_calls
            .lock()
            .expect("json media calls")
            .push((method, value));
    } else {
        let text = String::from_utf8_lossy(&body).to_string();
        state
            .multipart_calls
            .lock()
            .expect("multipart calls")
            .push((method, text));
    }

    (
        StatusCode::OK,
        Json(json!({"ok": true, "result": {"message_id": 9}})),
    )
}

fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: &std::sync::OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .blocking_lock()
}

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: &std::sync::OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock().await
}

#[tokio::test]
async fn telegram_loopback_covers_reaction_text_fallback_and_media_send_paths() {
    let _env = env_lock_async().await;
    let (base, state) = spawn_telegram_mock().await;
    let _guard = EnvVarGuard::set("OPENHUMAN_TELEGRAM_BOT_API_BASE", base);
    let _legacy_guard = EnvVarGuard::unset("OPENHUMAN_TELEGRAM_API_BASE");
    let channel = TelegramChannel::new("round26".to_string(), vec!["alice".to_string()], false);

    channel
        .send(&SendMessage::new("[REACTION:👍|42]", "chat-1:topic-2"))
        .await
        .expect("reaction-only send");
    channel
        .send(&SendMessage::new("[REACTION:💥|43]", "chat-1:topic-2"))
        .await
        .expect("failed reaction is non-fatal");
    channel
        .send(
            &SendMessage::new("**markdown fallback**", "chat-1:topic-2")
                .in_thread(Some("41".to_string())),
        )
        .await
        .expect("send text falls back to plain");

    channel
        .send_document_by_url(
            "chat-1",
            Some("topic-2"),
            "https://files.example/doc.pdf",
            Some("doc"),
        )
        .await
        .expect("document by url");
    channel
        .send_photo_by_url("chat-1", None, "https://files.example/photo.png", None)
        .await
        .expect("photo by url");
    channel
        .send_video_by_url(
            "chat-1",
            Some("topic-2"),
            "https://files.example/video.mp4",
            Some("video"),
        )
        .await
        .expect("video by url");
    channel
        .send_audio_by_url("chat-1", None, "https://files.example/audio.mp3", None)
        .await
        .expect("audio by url");
    channel
        .send_voice_by_url(
            "chat-1",
            Some("topic-2"),
            "https://files.example/voice.ogg",
            Some("voice"),
        )
        .await
        .expect("voice by url");
    channel
        .send_document_bytes(
            "chat-1",
            Some("topic-2"),
            b"round26 document".to_vec(),
            "round26.txt",
            Some("bytes"),
        )
        .await
        .expect("document bytes");
    channel
        .send_photo_bytes(
            "chat-1",
            None,
            b"not really an image".to_vec(),
            "round26.png",
            Some("photo bytes"),
        )
        .await
        .expect("photo bytes");

    let reactions = state.reaction_calls.lock().expect("reaction calls");
    assert_eq!(reactions.len(), 2);
    assert_eq!(reactions[0]["message_id"], 42);
    drop(reactions);

    let messages = state.send_message_calls.lock().expect("sendMessage calls");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["parse_mode"], "Markdown");
    assert!(messages[1].get("parse_mode").is_none());
    assert_eq!(messages[1]["message_thread_id"], "topic-2");
    assert_eq!(messages[1]["reply_to_message_id"], 41);
    drop(messages);

    let json_media = state.json_media_calls.lock().expect("json media calls");
    assert_eq!(json_media.len(), 5);
    assert_eq!(json_media[0].0, "sendDocument");
    assert_eq!(json_media[0].1["document"], "https://files.example/doc.pdf");
    assert_eq!(json_media[2].0, "sendVideo");
    assert_eq!(json_media[4].0, "sendVoice");
    drop(json_media);

    let multipart = state.multipart_calls.lock().expect("multipart calls");
    assert_eq!(multipart.len(), 2);
    assert_eq!(multipart[0].0, "sendDocument");
    assert!(multipart[0].1.contains("round26.txt"));
    assert_eq!(multipart[1].0, "sendPhoto");
    assert!(multipart[1].1.contains("round26.png"));
}
