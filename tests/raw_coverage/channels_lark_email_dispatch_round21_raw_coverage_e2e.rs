//! Round21 raw integration coverage for channels provider seams.
//!
//! Loopback servers and parser fixtures only: no real Discord, Lark, IMAP, or
//! SMTP traffic is performed.

use axum::{extract::Path, http::StatusCode, routing::get, Json, Router};
use openhuman_core::openhuman::channels::providers::discord::api::test_support as discord_support;
use openhuman_core::openhuman::channels::providers::email_channel::{
    test_support as email_support, EmailChannel, EmailConfig,
};
use openhuman_core::openhuman::channels::providers::lark::test_support as lark_support;
use openhuman_core::openhuman::channels::test_support::{
    run_dispatch_harness, DispatchHarnessOptions, TestMemoryEntry,
};
use openhuman_core::openhuman::channels::LarkChannel;
use reqwest::StatusCode as ReqwestStatusCode;
use serde_json::json;
// Lark's WS seam lives in tinychannels (tungstenite 0.29); use its re-export so
// the message type matches the function signature across the version boundary.
use tinychannels::tokio_tungstenite::tungstenite::Message as WsMsg;

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

#[test]
fn lark_parser_support_covers_post_mentions_placeholders_and_webhook_payloads() {
    let rich_post = json!({
        "fr_fr": {
            "title": "Incident digest",
            "content": [[
                {"tag": "text", "text": "See "},
                {"tag": "a", "href": "https://status.example.test/42"},
                {"tag": "at", "user_id": "ou_alice"},
                {"tag": "unknown", "text": "ignored"}
            ]]
        }
    })
    .to_string();
    let post_text = lark_support::parse_post_content_for_test(&rich_post).expect("post text");
    assert!(post_text.contains("Incident digest"));
    assert!(post_text.contains("https://status.example.test/42"));
    assert!(post_text.contains("@ou_alice"));
    assert!(lark_support::parse_post_content_for_test("{}").is_none());

    assert_eq!(
        lark_support::strip_at_placeholders_for_test("@_user_1 please review @_user_99 now"),
        "please review now"
    );
    assert!(!lark_support::should_respond_in_group_for_test(&[]));
    assert!(lark_support::should_respond_in_group_for_test(&[
        json!({"name": "OpenHuman"})
    ]));
    assert!(lark_support::should_refresh_last_recv_for_test(
        &WsMsg::Binary(vec![1, 2, 3].into())
    ));
    assert!(!lark_support::should_refresh_last_recv_for_test(
        &WsMsg::Text("not heartbeat".into())
    ));

    let channel = LarkChannel::new(
        "app".into(),
        "secret".into(),
        "verify".into(),
        None,
        vec!["ou_allowed".into()],
    );
    let payload = json!({
        "header": {"event_type": "im.message.receive_v1"},
        "event": {
            "sender": {"sender_id": {"open_id": "ou_allowed"}},
            "message": {
                "message_type": "post",
                "content": rich_post,
                "chat_id": "oc_round21",
                "create_time": "1700000000123"
            }
        }
    });
    let messages = channel.parse_event_payload(&payload);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].channel, "lark");
    assert_eq!(messages[0].reply_target, "oc_round21");
    assert_eq!(messages[0].timestamp, 1_700_000_000);
    assert!(messages[0].content.contains("Incident digest"));
}

#[test]
fn email_parser_support_covers_text_html_attachment_and_message_building() {
    let text_raw = b"From: Alice <alice@example.com>\r\nSubject: Plain\r\nMessage-ID: <plain-1@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from plain text.\r\n";
    let parsed = email_support::parse_email_fixture(text_raw).expect("plain parse");
    assert_eq!(parsed.sender, "alice@example.com");
    assert_eq!(parsed.subject.as_deref(), Some("Plain"));
    assert!(parsed.text.contains("Hello from plain text."));

    let html_raw = b"From: Bob <bob@example.com>\r\nSubject: HTML\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<div>Hello <strong>HTML</strong> body</div>\r\n";
    let parsed_html = email_support::parse_email_fixture(html_raw).expect("html parse");
    assert_eq!(parsed_html.sender, "bob@example.com");
    assert_eq!(parsed_html.text, "Hello HTML body");

    let attachment_raw = b"From: Ops <ops@example.com>\r\nSubject: Attachment\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=\"note.txt\"\r\n\r\nattachment text body\r\n--b--\r\n";
    let parsed_attachment =
        email_support::parse_email_fixture(attachment_raw).expect("attachment parse");
    assert!(parsed_attachment.text.contains("[Attachment: note.txt]"));
    assert!(parsed_attachment.text.contains("attachment text body"));

    let channel = EmailChannel::new(EmailConfig {
        from_address: "bot@example.com".into(),
        allowed_senders: vec!["@example.com".into(), "trusted.test".into()],
        ..Default::default()
    });
    assert!(channel.is_sender_allowed("ALICE@example.com"));
    assert!(channel.is_sender_allowed("person@trusted.test"));
    assert!(!channel.is_sender_allowed("person@untrusted.test"));

    let message = channel
        .build_plain_message("listener@example.com", "Round21", "coverage body")
        .expect("build message");
    let wire = String::from_utf8_lossy(&message.formatted()).to_string();
    assert!(wire.contains("Subject: Round21"));
    assert!(wire.contains("coverage body"));
}

#[tokio::test]
async fn discord_loopback_support_covers_lists_auth_errors_and_permission_overwrites() {
    let guild_app = Router::new().route(
        "/users/@me/guilds",
        get(|| async {
            Json(json!([
                {"id": "g2", "name": "Guild Two", "icon": null},
                {"id": "g1", "name": "Guild One", "icon": "hash"}
            ]))
        }),
    );
    let base = spawn_mock(guild_app).await;
    let guilds = discord_support::list_bot_guilds_at_base_for_test(&base, "token")
        .await
        .expect("guilds");
    assert_eq!(guilds.len(), 2);
    assert_eq!(guilds[1].icon.as_deref(), Some("hash"));

    let channel_app = Router::new().route(
        "/guilds/{guild_id}/channels",
        get(|Path(guild_id): Path<String>| async move {
            assert_eq!(guild_id, "g1");
            Json(json!([
                {"id": "voice", "name": "Voice", "type": 2, "position": 0, "parent_id": null},
                {"id": "late", "name": "Late", "type": 0, "position": 4, "parent_id": "cat"},
                {"id": "early", "name": "Early", "type": 0, "position": 1, "parent_id": null}
            ]))
        }),
    );
    let base = spawn_mock(channel_app).await;
    let channels = discord_support::list_guild_channels_at_base_for_test(&base, "token", "g1")
        .await
        .expect("channels");
    assert_eq!(
        channels.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["early", "late"]
    );

    let auth_error = discord_support::format_discord_http_error_for_test(
        "list_guilds",
        ReqwestStatusCode::UNAUTHORIZED,
        r#"{"message":"401: Unauthorized"}"#,
    );
    let lower = auth_error.to_ascii_lowercase();
    assert!(!lower.contains("401"));
    assert!(!lower.contains("unauthorized"));
    assert!(auth_error.contains("Settings"));

    let permission_app = Router::new()
        .route("/users/@me", get(|| async { Json(json!({"id": "bot-1"})) }))
        .route(
            "/guilds/{guild_id}/members/{member_id}",
            get(
                |Path((_guild_id, member_id)): Path<(String, String)>| async move {
                    assert_eq!(member_id, "bot-1");
                    Json(json!({"roles": ["role-send"], "user": {"id": "bot-1"}}))
                },
            ),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|Path(guild_id): Path<String>| async move {
                Json(json!([
                    {"id": guild_id, "permissions": "1024"},
                    {"id": "role-send", "permissions": "2048"}
                ]))
            }),
        )
        .route(
            "/channels/{channel_id}",
            get(|Path(channel_id): Path<String>| async move {
                assert_eq!(channel_id, "c1");
                Json(json!({
                    "permission_overwrites": [
                        {"id": "role-send", "type": 0, "allow": "65536", "deny": "2048"},
                        {"id": "bot-1", "type": 1, "allow": "2048", "deny": "0"}
                    ]
                }))
            }),
        );
    let base = spawn_mock(permission_app).await;
    let check =
        discord_support::check_channel_permissions_at_base_for_test(&base, "token", "g1", "c1")
            .await
            .expect("permission check");
    assert!(check.can_view_channel);
    assert!(check.can_send_messages);
    assert!(check.can_read_message_history);
    assert!(check.missing_permissions.is_empty());

    let failing_app = Router::new().route(
        "/users/@me",
        get(|| async { (StatusCode::BAD_GATEWAY, "discord unavailable") }),
    );
    let base = spawn_mock(failing_app).await;
    let err =
        discord_support::check_channel_permissions_at_base_for_test(&base, "token", "g1", "c1")
            .await
            .expect_err("me lookup fails")
            .to_string();
    assert!(err.contains("get_bot_user"));
    assert!(err.contains("502"));
}

#[tokio::test]
async fn dispatch_harness_round21_covers_non_web_context_success_and_timeout() {
    let observed = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "lark".to_string(),
        content: "thanks, can you summarize remembered channel state?".to_string(),
        thread_ts: Some("lark-thread".to_string()),
        supports_reactions: true,
        memory_entries: vec![TestMemoryEntry {
            key: "round21".to_string(),
            content: "Lark messages include reply target context.".to_string(),
            score: Some(0.95),
        }],
        response_text: Some("lark dispatch response".to_string()),
        ..Default::default()
    })
    .await;
    assert_eq!(observed.handler_channel_name, "lark");
    assert!(observed.handler_history_text.contains("[Channel context]"));
    assert!(observed.handler_history_text.contains("[Memory context]"));
    assert!(observed
        .sends
        .iter()
        .any(|send| send.kind == "send" && send.content.starts_with("[REACTION:")));
    assert!(observed
        .sends
        .iter()
        .any(|send| send.content == "lark dispatch response"));

    let timed_out = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "email".to_string(),
        content: "force timeout".to_string(),
        handler_delay_ms: 1_200,
        timeout_secs: 1,
        ..Default::default()
    })
    .await;
    assert!(timed_out
        .sends
        .iter()
        .any(|send| send.content.contains("Request timed out")));
}
