//! Round26 raw integration coverage for high-yield channel cold paths.
//!
//! Loopback Bot API endpoints and parser/codec fixtures only: no real channel
//! network services are contacted.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use openhuman_core::openhuman::channels::providers::email_channel::{
    test_support as email_support, EmailChannel, EmailConfig,
};
use openhuman_core::openhuman::channels::providers::irc::test_support as irc_support;
use openhuman_core::openhuman::channels::providers::telegram::TelegramChannel;
use openhuman_core::openhuman::channels::providers::yuanbao::{
    proto::decode_conn_msg,
    proto_biz::{
        decode_biz_rsp_code, decode_get_group_member_list_rsp, decode_query_group_info_rsp,
        decode_response_envelope, encode_get_group_member_list, encode_query_group_info,
        encode_send_c2c_message, encode_send_group_heartbeat, encode_send_group_message,
        encode_send_private_heartbeat,
    },
    proto_constants::{biz_cmd, cmd_type, module, ws_heartbeat},
    types::{MsgBodyElement, MsgContent},
    wire::{encode_field_bytes, encode_field_string, encode_field_varint},
};
use openhuman_core::openhuman::channels::traits::{Channel, SendMessage};
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

struct EnvGuard {
    key: &'static str,
    prior: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let prior = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prior }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.prior.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

impl EnvGuard {
    fn unset(key: &'static str) -> Self {
        let prior = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prior }
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn text_body(text: &str) -> Vec<MsgBodyElement> {
    vec![MsgBodyElement {
        msg_type: "TIMTextElem".to_string(),
        msg_content: MsgContent {
            text: Some(text.to_string()),
            ..Default::default()
        },
    }]
}

#[tokio::test]
async fn telegram_loopback_covers_reaction_text_fallback_and_media_send_paths() {
    let _env = env_lock();
    let (base, state) = spawn_telegram_mock().await;
    let _guard = EnvGuard::set("OPENHUMAN_TELEGRAM_BOT_API_BASE", base);
    let _legacy_guard = EnvGuard::unset("OPENHUMAN_TELEGRAM_API_BASE");
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

#[test]
fn irc_and_email_parser_edges_cover_helpers_without_sockets() {
    let parsed = irc_support::parse_line_for_test(":Alice!u@h PRIVMSG #ops :hello world")
        .expect("irc privmsg parse");
    assert_eq!(parsed.0.as_deref(), Some("Alice!u@h"));
    assert_eq!(parsed.1, "PRIVMSG");
    assert_eq!(
        parsed.2,
        vec!["#ops".to_string(), "hello world".to_string()]
    );
    assert_eq!(parsed.3.as_deref(), Some("Alice"));
    assert_eq!(
        irc_support::parse_line_for_test("PING :server")
            .expect("ping parse")
            .2,
        vec!["server".to_string()]
    );
    assert!(irc_support::parse_line_for_test("").is_none());
    assert_eq!(
        irc_support::encode_sasl_plain_for_test("openhuman", "secret"),
        "AG9wZW5odW1hbgBzZWNyZXQ="
    );
    assert!(irc_support::is_user_allowed_for_test(
        vec!["*".to_string()],
        "Anyone"
    ));
    assert!(irc_support::is_user_allowed_for_test(
        vec!["Alice".to_string()],
        "alice"
    ));
    assert!(!irc_support::is_user_allowed_for_test(
        vec!["Alice".to_string()],
        "bob"
    ));

    let chunks = irc_support::split_message_for_test("alpha\nβeta\r\n0123456789", 5);
    assert_eq!(chunks, vec!["alpha", "βeta", "01234", "56789"]);
    assert_eq!(
        irc_support::split_message_for_test("\n", 0),
        vec![String::new()]
    );

    let channel = EmailChannel::new(EmailConfig {
        from_address: "bot@example.test".to_string(),
        allowed_senders: vec![
            "*".to_string(),
            "admin@example.test".to_string(),
            "@team.example".to_string(),
        ],
        ..Default::default()
    });
    assert!(channel.is_sender_allowed("blocked@anywhere.test"));
    assert_eq!(
        EmailChannel::strip_html("<div>hello<br><span>team</span></div>"),
        "helloteam"
    );
    let no_body = b"From: Unknown <nobody@example.test>\r\nSubject: No Body\r\n\r\n";
    let parsed = email_support::parse_email_fixture(no_body).expect("email parse");
    assert_eq!(parsed.sender, "nobody@example.test");
    assert_eq!(parsed.subject.as_deref(), Some("No Body"));
    assert!(parsed.text.is_empty());
    let plain = channel
        .build_plain_message("ops@example.test", "Round26", "plain body")
        .expect("plain email");
    let formatted = String::from_utf8_lossy(&plain.formatted()).to_string();
    assert!(formatted.contains("Subject: Round26"));
    assert!(formatted.contains("plain body"));
}

#[test]
fn yuanbao_biz_codecs_cover_success_error_and_optional_field_paths() {
    let c2c = encode_send_c2c_message(
        "uid_alice",
        "uid_bot",
        &text_body("hello dm"),
        "msg-c2c",
        99,
        "group-from-dm",
        "trace-c2c",
    );
    let c2c_frame = decode_conn_msg(&c2c).expect("decode c2c frame");
    assert_eq!(c2c_frame.cmd_type, cmd_type::REQUEST);
    assert_eq!(c2c_frame.cmd, biz_cmd::SEND_C2C_MESSAGE);
    assert_eq!(c2c_frame.module, module::BIZ_PKG);
    assert_eq!(c2c_frame.msg_id, "msg-c2c");

    let group = encode_send_group_message(
        "group-1",
        "uid_bot",
        &text_body("hello group"),
        "msg-group",
        "uid-target",
        "random-1",
        "ref-1",
        "trace-group",
    );
    let group_frame = decode_conn_msg(&group).expect("decode group frame");
    assert_eq!(group_frame.cmd, biz_cmd::SEND_GROUP_MESSAGE);
    assert_eq!(group_frame.msg_id, "msg-group");

    let private_hb =
        encode_send_private_heartbeat("hb-private", "uid_bot", "uid_alice", ws_heartbeat::RUNNING);
    assert_eq!(
        decode_conn_msg(&private_hb).expect("private hb").cmd,
        biz_cmd::SEND_PRIVATE_HEARTBEAT
    );
    let group_hb = encode_send_group_heartbeat(
        "hb-group",
        "uid_bot",
        "group-1",
        ws_heartbeat::RUNNING,
        1_700_000_001,
    );
    assert_eq!(
        decode_conn_msg(&group_hb).expect("group hb").cmd,
        biz_cmd::SEND_GROUP_HEARTBEAT
    );

    let group_info_req = encode_query_group_info("q-info", "group-1");
    assert_eq!(
        decode_response_envelope(&group_info_req)
            .expect("query group envelope")
            .cmd,
        biz_cmd::QUERY_GROUP_INFO
    );
    let member_req = encode_get_group_member_list("q-members", "group-1", 20, 10);
    assert_eq!(
        decode_conn_msg(&member_req).expect("member req").cmd,
        biz_cmd::GET_GROUP_MEMBER_LIST
    );

    let mut group_inner = Vec::new();
    encode_field_string(1, "Round26 Group", &mut group_inner);
    encode_field_string(2, "owner-id", &mut group_inner);
    encode_field_string(3, "Owner", &mut group_inner);
    encode_field_varint(4, 3, &mut group_inner);
    let mut group_rsp = Vec::new();
    encode_field_varint(1, 0, &mut group_rsp);
    encode_field_string(2, "ok", &mut group_rsp);
    encode_field_bytes(3, &group_inner, &mut group_rsp);
    let group_info = decode_query_group_info_rsp(&group_rsp).expect("group info rsp");
    assert_eq!(group_info.group_name, "Round26 Group");
    assert_eq!(group_info.member_count, 3);

    let mut member = Vec::new();
    encode_field_string(1, "uid-member", &mut member);
    encode_field_string(2, "Member", &mut member);
    encode_field_varint(3, 2, &mut member);
    encode_field_varint(4, 1_700_000_000, &mut member);
    encode_field_string(5, "Card", &mut member);
    let mut members_rsp = Vec::new();
    encode_field_varint(1, 0, &mut members_rsp);
    encode_field_string(2, "ok", &mut members_rsp);
    encode_field_bytes(3, &member, &mut members_rsp);
    encode_field_varint(4, 30, &mut members_rsp);
    encode_field_varint(5, 1, &mut members_rsp);
    let page = decode_get_group_member_list_rsp(&members_rsp).expect("members rsp");
    assert_eq!(page.members[0].user_id, "uid-member");
    assert_eq!(page.members[0].name_card, "Card");
    assert_eq!(page.next_offset, 30);
    assert!(page.is_complete);

    let mut error_rsp = Vec::new();
    encode_field_varint(1, 4002, &mut error_rsp);
    encode_field_string(2, "rate limited", &mut error_rsp);
    assert_eq!(
        decode_biz_rsp_code(&error_rsp).expect("biz code"),
        (4002, "rate limited".to_string())
    );
    let mut overflow = Vec::new();
    encode_field_varint(1, u64::MAX, &mut overflow);
    assert!(decode_biz_rsp_code(&overflow)
        .expect_err("overflow rejected")
        .to_string()
        .contains("out of i32 range"));
}
