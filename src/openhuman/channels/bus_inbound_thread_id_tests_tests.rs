use super::{
    channel_message_body_with_idempotency, channel_supports_progressive_ui,
    derive_inbound_client_id, derive_inbound_thread_id,
};
use serde_json::json;

#[test]
fn progressive_ui_is_an_allowlist_failing_safe_for_unknown_channels() {
    // Only edit+delete-capable providers opt in. Telegram supports both;
    // everything else (Discord's stub delete / 404 edits, and any new or
    // unknown adapter) is suppressed so the "💭" spam can't reappear.
    assert!(channel_supports_progressive_ui("telegram"));
    assert!(channel_supports_progressive_ui("tg"));
    // Inbound channels arrive provider-prefixed — the prefix must still match.
    assert!(channel_supports_progressive_ui("tg:12345"));
    assert!(!channel_supports_progressive_ui("discord"));
    assert!(!channel_supports_progressive_ui("discord:guild-1"));
    // Unknown/new adapters fail safe (allowlist, not denylist).
    assert!(!channel_supports_progressive_ui("slack"));
    assert!(!channel_supports_progressive_ui("whatsapp:123"));
}

#[test]
fn channel_message_body_adds_deterministic_idempotency_key() {
    let left = channel_message_body_with_idempotency(
        "telegram",
        json!({ "text": "hello", "threadId": "topic-1" }),
    );
    let right = channel_message_body_with_idempotency(
        "telegram",
        json!({ "threadId": "topic-1", "text": "hello" }),
    );

    assert_eq!(left["text"], "hello");
    assert_eq!(left["threadId"], "topic-1");
    assert_eq!(left["idempotencyKey"], right["idempotencyKey"]);
    assert!(left["idempotencyKey"]
        .as_str()
        .expect("idempotency key")
        .starts_with("legacy-send:telegram:"));
}

#[test]
fn channel_message_body_preserves_caller_idempotency_key() {
    let body = channel_message_body_with_idempotency(
        "discord",
        json!({ "text": "hello", "idempotencyKey": "caller-key" }),
    );

    assert_eq!(body["idempotencyKey"], "caller-key");
}

#[test]
fn socket_inbound_client_id_keys_per_sender() {
    // Distinct senders in the same shared channel must produce distinct
    // client_id labels so downstream consumers that key on client_id
    // (audit log, future session caches) stay segregated. The
    // thread_id is already per-sender; this is the matching client_id
    // half of the pair.
    let alice = derive_inbound_client_id("discord", Some("alice"));
    let bob = derive_inbound_client_id("discord", Some("bob"));
    assert_ne!(alice, bob, "co-channel senders must not collapse");
    assert!(alice.starts_with("inbound"));
    assert!(bob.starts_with("inbound"));
}

#[test]
fn socket_inbound_client_id_legacy_fallback_keeps_bare_inbound() {
    // Legacy publishers that don't fill `sender` keep the historical
    // `"inbound"` literal so single-DM flows (where there's no
    // co-channel surface) are unchanged.
    assert_eq!(derive_inbound_client_id("discord", None), "inbound");
    assert_eq!(derive_inbound_client_id("discord", Some("")), "inbound");
    assert_eq!(derive_inbound_client_id("discord", Some("   ")), "inbound");
}

#[test]
fn socket_inbound_keys_per_sender_combined_with_thread_id() {
    // Regression: in a shared Discord channel, two distinct senders
    // sending into the same channel/reply_target produce a fully
    // distinct (client_id, thread_id) pair. This is the surface the
    // wallet preparer-binding and parked-approval routing both rely
    // on for per-user isolation.
    let alice_thread = derive_inbound_thread_id("discord", Some("alice"), Some("#general"), None);
    let bob_thread = derive_inbound_thread_id("discord", Some("bob"), Some("#general"), None);
    let alice_client = derive_inbound_client_id("discord", Some("alice"));
    let bob_client = derive_inbound_client_id("discord", Some("bob"));

    assert_ne!(alice_thread, bob_thread);
    assert_ne!(alice_client, bob_client);
    assert_ne!(
        (alice_client.as_str(), alice_thread.as_str()),
        (bob_client.as_str(), bob_thread.as_str()),
    );
}

#[test]
fn legacy_channel_only_keeps_old_shape() {
    // Publishers that don't pass sender must still produce a stable
    // key so existing single-DM flows are unchanged.
    assert_eq!(
        derive_inbound_thread_id("telegram", None, None, None),
        "channel:telegram"
    );
}

#[test]
fn distinct_senders_get_distinct_keys() {
    let a = derive_inbound_thread_id("discord", Some("alice"), Some("#general"), None);
    let b = derive_inbound_thread_id("discord", Some("bob"), Some("#general"), None);
    assert_ne!(a, b, "two senders in same channel must not collapse");
}

#[test]
fn slack_thread_anchor_splits_subthreads() {
    let parent = derive_inbound_thread_id("slack", Some("u1"), Some("C1"), None);
    let thread = derive_inbound_thread_id("slack", Some("u1"), Some("C1"), Some("1700.001"));
    assert_ne!(parent, thread);
}

#[test]
fn telegram_ignores_thread_ts() {
    // Telegram uses thread_ts for transport routing only; memory key
    // must stay stable across thread_ts updates inside the same DM.
    let a = derive_inbound_thread_id("telegram", Some("u1"), Some("c1"), Some("100"));
    let b = derive_inbound_thread_id("telegram", Some("u1"), Some("c1"), Some("200"));
    assert_eq!(a, b);
}

#[test]
fn telegram_chat_id_shape_still_ignores_thread_ts() {
    // Regression: in production the socket layer addresses Telegram
    // with raw chat ids like `tg:123` and `telegram:123` (matching
    // the `<provider>:message` event name shape). The thread_ts
    // carve-out must recognise both, not only the literal slug.
    for channel in ["tg:123", "telegram:123", "tg", "telegram"] {
        let a = derive_inbound_thread_id(channel, Some("u1"), Some("c1"), Some("100"));
        let b = derive_inbound_thread_id(channel, Some("u1"), Some("c1"), Some("200"));
        assert_eq!(
            a, b,
            "channel '{channel}' should ignore thread_ts (telegram provider)"
        );
    }
}

#[test]
fn non_telegram_channel_id_shape_still_splits_on_thread_ts() {
    // Inverse: a `slack:<workspace>` style channel must continue to
    // honour thread_ts so Slack subthreads stay distinct.
    let a = derive_inbound_thread_id("slack:T1", Some("u1"), Some("c1"), Some("100"));
    let b = derive_inbound_thread_id("slack:T1", Some("u1"), Some("c1"), Some("200"));
    assert_ne!(a, b);
}

#[test]
fn empty_optional_fields_are_skipped() {
    let only_sender = derive_inbound_thread_id("discord", Some("alice"), Some("   "), None);
    assert_eq!(only_sender, "channel:discord/alice");
}
