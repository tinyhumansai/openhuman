use super::*;
use tinyflows_bus::schedule::ChannelConversationIdentity;

#[test]
fn conversation_ref_preserves_full_provider_scope() {
    let identity = ChannelConversationIdentity {
        kind: "group".into(),
        id: "room-1".into(),
        scope_id: Some("account-scope".into()),
        parent_id: Some("parent-1".into()),
        thread_id: Some("thread-2".into()),
        topic_id: Some("topic-3".into()),
    };
    let conversation = conversation_ref(&identity).unwrap();
    assert_eq!(conversation.kind, ConversationKind::Group);
    assert_eq!(conversation.id, "room-1");
    assert_eq!(conversation.scope_id.as_deref(), Some("account-scope"));
    assert_eq!(conversation.parent_id.as_deref(), Some("parent-1"));
    assert_eq!(conversation.thread_id.as_deref(), Some("thread-2"));
    assert_eq!(conversation.topic_id.as_deref(), Some("topic-3"));
}

#[test]
fn conversation_ref_rejects_unknown_kind_claims() {
    let identity = ChannelConversationIdentity {
        kind: "invented".into(),
        id: "room-1".into(),
        scope_id: None,
        parent_id: None,
        thread_id: None,
        topic_id: None,
    };
    assert!(conversation_ref(&identity).is_err());
}

fn session(
    provider: &str,
    account: Option<&str>,
    conversation: &str,
) -> tinyflows_bus::schedule::ChannelSessionIdentity {
    tinyflows_bus::schedule::ChannelSessionIdentity {
        tenant_key: "t".into(),
        agent_id: "orchestrator".into(),
        session_key: "s".into(),
        provider: provider.into(),
        account_id: account.map(str::to_owned),
        conversation: ChannelConversationIdentity {
            kind: "dm".into(),
            id: conversation.into(),
            scope_id: None,
            parent_id: None,
            thread_id: None,
            topic_id: None,
        },
    }
}

#[tokio::test]
async fn sends_and_announces_fail_closed_without_a_running_runtime() {
    clear_channel_bridge();
    let id = session("telegram", Some("telegram"), "42");
    assert_eq!(
        send_to_channel("telegram", &id, "42", "hi", "k")
            .await
            .unwrap_err(),
        "channel runtime is not running"
    );
    assert_eq!(
        announce_to_channel("telegram", "42", None, "hi", "k")
            .await
            .unwrap_err(),
        "channel runtime is not running"
    );
}

#[tokio::test]
async fn send_rejects_identities_that_do_not_match_the_trusted_route() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let objects = linked.module_objects();
    let account = objects
        .keys()
        .find(|k| tinychannels_bus::provider_id(k) == "telegram")
        .unwrap()
        .clone();
    register_channel_bridge(objects);

    let mismatch = send_to_channel(
        "telegram",
        &session("slack", Some(&account), "42"),
        "42",
        "x",
        "k",
    )
    .await;
    assert_eq!(
        mismatch.unwrap_err(),
        "channel origin provider does not match the captured session"
    );

    let no_account =
        send_to_channel("telegram", &session("telegram", None, "42"), "42", "x", "k").await;
    assert_eq!(
        no_account.unwrap_err(),
        "channel origin has no configured account identity"
    );

    let foreign_account = send_to_channel(
        "telegram",
        &session("telegram", Some("slack:team"), "42"),
        "42",
        "x",
        "k",
    )
    .await;
    assert_eq!(
        foreign_account.unwrap_err(),
        "channel origin has no configured account identity"
    );

    let other_room = send_to_channel(
        "telegram",
        &session("telegram", Some(&account), "42"),
        "99",
        "x",
        "k",
    )
    .await;
    assert_eq!(
        other_room.unwrap_err(),
        "channel origin recipient does not match the captured conversation"
    );

    let missing = send_to_channel(
        "telegram",
        &session("telegram", Some("telegram:ghost"), "42"),
        "42",
        "x",
        "k",
    )
    .await;
    assert_eq!(
        missing.unwrap_err(),
        "configured channel module is unavailable"
    );

    let mut bad_kind = session("telegram", Some(&account), "42");
    bad_kind.conversation.kind = "invented".into();
    let invalid = send_to_channel("telegram", &bad_kind, "42", "x", "k").await;
    assert_eq!(
        invalid.unwrap_err(),
        "channel origin conversation kind is invalid"
    );

    let unconfigured = announce_to_channel("discord", "42", Some("thread"), "x", "k").await;
    assert_eq!(
        unconfigured.unwrap_err(),
        "channel discord is not configured"
    );

    let ok = send_to_channel(
        "telegram",
        &session("telegram", Some(&account), "42"),
        "42",
        "sent",
        "k-ok",
    )
    .await;
    clear_channel_bridge();
    ok.expect("a matching identity is delivered through the module");
    assert!(linked
        .provider_request_paths()
        .await
        .iter()
        .any(|p| p.ends_with("/sendMessage")));
}
