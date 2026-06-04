//! Round25 raw integration coverage for large channel misses.
//!
//! Only loopback services and parser fixtures are used.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use openhuman_core::core::socketio::WebChannelEvent;
use openhuman_core::openhuman::channels::providers::email_channel::{
    test_support as email_support, EmailChannel, EmailConfig,
};
use openhuman_core::openhuman::channels::providers::lark::test_support as lark_support;
use openhuman_core::openhuman::channels::providers::mattermost::{
    test_support as mattermost_support, MattermostChannel,
};
use openhuman_core::openhuman::channels::providers::telegram::test_support as telegram_support;
use openhuman_core::openhuman::channels::providers::web::{self, test_support as web_support};
use openhuman_core::openhuman::channels::test_support::{
    build_channel_context_block_for_test, run_dispatch_harness,
    select_acknowledgment_reaction_for_test, DispatchHarnessOptions, TestMemoryEntry,
};
use openhuman_core::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast::error::RecvError;

#[derive(Default)]
struct MattermostMockState {
    post_bodies: Mutex<Vec<Value>>,
    typing_bodies: Mutex<Vec<Value>>,
    auth_headers: Mutex<Vec<String>>,
}

async fn spawn_mattermost_mock() -> (String, Arc<MattermostMockState>) {
    let state = Arc::new(MattermostMockState::default());
    let app = Router::new()
        .route(
            "/api/v4/users/me",
            get(|| async { Json(json!({"id": "bot-id", "username": "openhuman"})) }),
        )
        .route(
            "/api/v4/posts",
            post(
                |State(state): State<Arc<MattermostMockState>>,
                 headers: HeaderMap,
                 Json(body): Json<Value>| async move {
                    if let Some(auth) = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                    {
                        state.auth_headers.lock().expect("auth headers").push(auth.to_string());
                    }
                    state.post_bodies.lock().expect("post bodies").push(body);
                    (StatusCode::OK, Json(json!({"id": "post-created"})))
                },
            ),
        )
        .route(
            "/api/v4/users/me/typing",
            post(
                |State(state): State<Arc<MattermostMockState>>,
                 Json(body): Json<Value>| async move {
                    state.typing_bodies.lock().expect("typing bodies").push(body);
                    (StatusCode::OK, Json(json!({"ok": true})))
                },
            ),
        )
        .with_state(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mattermost mock");
    let addr = listener.local_addr().expect("mattermost mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve mattermost mock");
    });
    (format!("http://127.0.0.1:{}", addr.port()), state)
}

async fn recv_event(
    rx: &mut tokio::sync::broadcast::Receiver<WebChannelEvent>,
    expected: &str,
) -> WebChannelEvent {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("web event timeout")
            .unwrap_or_else(|err| match err {
                RecvError::Lagged(_) => panic!("web event receiver lagged"),
                RecvError::Closed => panic!("web event receiver closed"),
            });
        if event.event == expected {
            return event;
        }
    }
}

#[tokio::test]
async fn web_channel_validation_cancellation_and_error_events_are_observable() {
    assert_eq!(web_support::key_for_test("thread-1"), "thread-1");
    assert_eq!(
        serde_json::from_str::<Value>(&web_support::event_session_id_for_test(
            "client-1", "thread-1"
        ))
        .expect("session json"),
        json!({"client_id": "client-1", "thread_id": "thread-1"})
    );

    assert!(
        web::start_chat(" ", "thread", "hello", None, None, None, None, None)
            .await
            .unwrap_err()
            .contains("client_id is required")
    );
    assert!(web::cancel_chat("client", " ")
        .await
        .unwrap_err()
        .contains("thread_id"));

    let mut rx = web::subscribe_web_channel_events();
    web_support::set_forced_run_chat_task_error_for_test(Some(
        "provider OpenAI returned 429 rate limit; retry after 2 seconds",
    ))
    .await;
    let request_id = web::start_chat(
        "client-round25",
        "thread-round25",
        "hello from round25",
        Some("gpt-5".to_string()),
        Some(0.2),
        None,
        Some("en-US".to_string()),
        None,
    )
    .await
    .expect("start forced-error chat");

    let event = recv_event(&mut rx, "chat_error").await;
    assert_eq!(event.client_id, "client-round25");
    assert_eq!(event.thread_id, "thread-round25");
    assert_eq!(event.request_id, request_id);
    assert_eq!(event.error_type.as_deref(), Some("rate_limited"));
    assert_eq!(event.error_retryable, Some(true));

    let missing = web::cancel_chat("client-round25", "thread-round25")
        .await
        .expect("cancel non-inflight");
    assert!(missing.is_none());

    // Clear the forced error so subsequent tests in this binary are not affected.
    web_support::set_forced_run_chat_task_error_for_test(None).await;
}

#[tokio::test]
async fn mattermost_loopback_send_typing_health_and_parser_paths() {
    let (base, state) = spawn_mattermost_mock().await;
    let channel = MattermostChannel::new(
        format!("{base}/"),
        "mm-token".to_string(),
        Some("chan-1".to_string()),
        vec!["alice".to_string()],
        true,
        true,
    );

    assert!(channel.health_check().await);
    channel
        .send(&SendMessage::new("hello mattermost", "chan-1:root-7"))
        .await
        .expect("send mattermost");
    channel
        .start_typing("chan-1:root-7")
        .await
        .expect("start typing");
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    channel
        .stop_typing("chan-1:root-7")
        .await
        .expect("stop typing");

    let post_bodies = state.post_bodies.lock().expect("post bodies");
    assert_eq!(post_bodies[0]["channel_id"], "chan-1");
    assert_eq!(post_bodies[0]["root_id"], "root-7");
    assert_eq!(post_bodies[0]["message"], "hello mattermost");
    drop(post_bodies);

    let typing_bodies = state.typing_bodies.lock().expect("typing bodies");
    assert_eq!(typing_bodies[0]["channel_id"], "chan-1");
    assert_eq!(typing_bodies[0]["parent_id"], "root-7");
    drop(typing_bodies);
    assert_eq!(
        state.auth_headers.lock().expect("auth headers")[0],
        "Bearer mm-token"
    );

    let post = json!({
        "id": "post-1",
        "user_id": "alice",
        "message": "@OpenHuman please triage",
        "create_at": 1_700_000_005_000_i64,
        "metadata": {"mentions": ["bot-id"]}
    });
    assert!(mattermost_support::contains_bot_mention_for_test(
        "hello @openhuman",
        "bot-id",
        "OpenHuman",
        &post,
    ));
    assert_eq!(
        mattermost_support::normalize_mattermost_content_for_test(
            "@OpenHuman please triage",
            "bot-id",
            "OpenHuman",
            &post,
        )
        .as_deref(),
        Some("please triage")
    );
    let parsed = mattermost_support::parse_mattermost_post_for_test(
        &channel,
        &post,
        "bot-id",
        "OpenHuman",
        1_700_000_000_000_i64,
        "chan-1",
    )
    .expect("mattermost parsed");
    assert_eq!(parsed.channel, "mattermost");
    assert_eq!(parsed.reply_target, "chan-1:post-1");
    assert_eq!(parsed.content, "please triage");

    let denied = mattermost_support::parse_mattermost_post_for_test(
        &MattermostChannel::new(
            base,
            "mm-token".to_string(),
            None,
            vec!["bob".to_string()],
            true,
            false,
        ),
        &post,
        "bot-id",
        "OpenHuman",
        1_700_000_000_000_i64,
        "chan-1",
    );
    assert!(denied.is_none());
}

#[test]
fn lark_email_telegram_and_dispatch_pure_paths_cover_large_helpers() {
    let (tenant_url, send_url) = lark_support::endpoint_urls_for_test(false);
    assert!(tenant_url.contains("open.larksuite.com/open-apis/auth"));
    assert!(send_url.ends_with("/im/v1/messages?receive_id_type=chat_id"));

    let (ws_url, ping) = lark_support::endpoint_response_for_test(
        r#"{"code":0,"data":{"URL":"wss://lark.example/ws?service_id=42","ClientConfig":{"PingInterval":11}}}"#,
    )
    .expect("endpoint parse");
    assert_eq!(ws_url, "wss://lark.example/ws?service_id=42");
    assert_eq!(ping, Some(11));
    assert!(
        lark_support::endpoint_response_for_test(r#"{"code":1901,"msg":"bad app"}"#)
            .unwrap_err()
            .to_string()
            .contains("1901")
    );

    let encoded =
        lark_support::encode_frame_for_test(7, 0, "pong", Some(br#"{"ok":true}"#.to_vec()));
    let decoded = lark_support::decode_frame_for_test(&encoded).expect("decode frame");
    assert_eq!(decoded.0, 7);
    assert_eq!(decoded.1, 0);
    assert_eq!(decoded.2, "pong");
    assert_eq!(decoded.3.as_deref(), Some(br#"{"ok":true}"#.as_slice()));

    let (remaining, reaction) =
        telegram_support::parse_reaction_marker_for_test(" [REACTION:👍|123] continuing ");
    assert_eq!(remaining, "continuing");
    assert_eq!(reaction.as_deref(), Some("👍|123"));
    assert_eq!(
        telegram_support::parse_reaction_marker_for_test("[REACTION:]").1,
        None
    );

    let raw = b"From: Nobody <nobody@example.com>\r\nSubject: Empty\r\n\r\n";
    let parsed = email_support::parse_email_fixture(raw).expect("empty email parse");
    assert_eq!(parsed.sender, "nobody@example.com");
    assert!(parsed.text.is_empty());
    let message = EmailChannel::new(EmailConfig {
        from_address: "bot@example.com".to_string(),
        ..Default::default()
    })
    .build_message_with_attachment(
        "ops@example.com",
        "Artifact",
        "body",
        "artifact.txt",
        lettre::message::header::ContentType::TEXT_PLAIN,
        b"artifact bytes".to_vec(),
    )
    .expect("message with attachment");
    let formatted = String::from_utf8_lossy(&message.formatted()).to_string();
    assert!(formatted.contains("Artifact"));
    assert!(formatted.contains("artifact.txt"));

    let web_msg = ChannelMessage {
        id: "m1".to_string(),
        sender: "alice".to_string(),
        content: "hello".to_string(),
        channel: "web".to_string(),
        reply_target: "thread".to_string(),
        timestamp: 1,
        thread_ts: None,
    };
    assert!(build_channel_context_block_for_test(&web_msg).is_empty());
    assert_eq!(
        select_acknowledgment_reaction_for_test("thank you")
            .chars()
            .count(),
        1
    );
}

#[tokio::test]
async fn dispatch_harness_round25_covers_streaming_error_and_history_compaction() {
    let observed = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "mattermost".to_string(),
        content: "can you debug this api issue?".to_string(),
        thread_ts: Some("thread-ts".to_string()),
        streaming: true,
        supports_reactions: false,
        seed_history_len: 40,
        memory_entries: vec![TestMemoryEntry {
            key: "round25-memory".to_string(),
            content: "Mattermost replies should preserve channel context.".to_string(),
            score: Some(0.9),
        }],
        response_text: Some("final streamed response".to_string()),
        ..Default::default()
    })
    .await;
    assert!(observed.handler_had_progress);
    assert!(observed.handler_history_text.contains("[Channel context]"));
    assert!(observed.handler_history_text.contains("[Memory context]"));
    assert!(observed.retained_history_len > 0);
    assert!(observed
        .sends
        .iter()
        .any(|send| send.kind == "finalize_draft" && send.content == "final streamed response"));

    let errored = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "telegram".to_string(),
        content: "force handler failure".to_string(),
        handler_error: Some("synthetic handler failure".to_string()),
        ..Default::default()
    })
    .await;
    assert!(errored
        .sends
        .iter()
        .any(|send| send.content.contains("synthetic handler failure")));
}
