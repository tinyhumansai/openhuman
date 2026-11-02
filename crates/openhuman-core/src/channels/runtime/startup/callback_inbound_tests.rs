use super::*;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

#[tokio::test]
async fn module_inbound_envelope_reaches_host_dispatch_with_identity_intact() {
    let (dispatch, mut received) = tokio::sync::mpsc::channel(1);
    let callback = ChannelHostCallbacks::new(
        &crate::core::runtime::DispatchCtx::host_default(),
        Arc::new(crate::config::Config::default()),
        Arc::new(OnceLock::new()),
        Arc::new(OnceLock::from(Arc::new(HashMap::new()))),
        None,
        dispatch,
    );
    let envelope = tinychannels_bus::ChannelInboundEnvelope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some("telegram:bot-a".into()),
        },
        message_id: "telegram_42_10".into(),
        conversation: tinychannels_bus::ConversationRef {
            kind: tinychannels_bus::ConversationKind::Dm,
            id: "42".into(),
            scope_id: Some("account-scope".into()),
            thread_id: Some("topic-8".into()),
            topic_id: Some("topic-8".into()),
            ..Default::default()
        },
        sender: tinychannels_bus::SenderRef {
            id: "42".into(),
            name: Some("Ada".into()),
            ..Default::default()
        },
        text: "native Telegram inbound".into(),
        reply_to_id: Some("parent-message-9".into()),
        timestamp_unix_ms: Some(1_700_000_000_123),
        access: tinychannels_bus::channel::AccessContext {
            dm_decision: tinychannels_bus::channel::SenderDmDecision::Allow,
            ..Default::default()
        },
        parts: vec![tinychannels_bus::ContentPart::Text {
            text: "native Telegram inbound".into(),
            markdown_dialect: None,
        }],
        raw: Some(serde_json::json!({ "providerMessageId": "10" })),
        ..Default::default()
    };

    callback
        .deliver_inbound(
            "telegram:bot-a".into(),
            serde_json::to_value(&envelope).expect("encode native envelope"),
        )
        .await
        .expect("native envelope should be accepted by host callback");

    let runtime_message = received
        .try_recv()
        .expect("native envelope should enter the channel dispatch queue");
    assert!(runtime_message.message.channel.is_empty());
    assert!(!runtime_message.authenticated_relay);
    let received_envelope = runtime_message
        .inbound_envelope
        .expect("typed source envelope must reach module preparation");
    assert_eq!(received_envelope.channel, envelope.channel);
    assert_eq!(received_envelope.conversation, envelope.conversation);
    assert_eq!(received_envelope.sender, envelope.sender);
    assert_eq!(received_envelope.message_id, envelope.message_id);
    assert_eq!(received_envelope.reply_to_id, envelope.reply_to_id);
    assert_eq!(
        received_envelope.timestamp_unix_ms,
        envelope.timestamp_unix_ms
    );
    assert_eq!(received_envelope.text, envelope.text);
    assert_eq!(received_envelope.parts, envelope.parts);
    assert_eq!(received_envelope.access, envelope.access);
    assert_eq!(received_envelope.raw, envelope.raw);
}

#[tokio::test]
async fn typed_inbound_rejects_malformed_or_cross_account_envelopes_without_queueing() {
    let (dispatch, mut received) = tokio::sync::mpsc::channel(4);
    let callback = ChannelHostCallbacks::new(
        &crate::core::runtime::DispatchCtx::host_default(),
        Arc::new(crate::config::Config::default()),
        Arc::new(OnceLock::new()),
        Arc::new(OnceLock::from(Arc::new(HashMap::new()))),
        None,
        dispatch,
    );
    let valid_envelope = || tinychannels_bus::ChannelInboundEnvelope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some("telegram:bot-a".into()),
        },
        message_id: "message-1".into(),
        conversation: tinychannels_bus::ConversationRef {
            kind: tinychannels_bus::ConversationKind::Dm,
            id: "42".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut wrong_account = valid_envelope();
    wrong_account.channel.account_id = Some("telegram:bot-b".into());
    let mut wrong_provider = valid_envelope();
    wrong_provider.channel.id = "slack".into();
    let mut empty_message_id = valid_envelope();
    empty_message_id.message_id.clear();
    let mut empty_conversation_id = valid_envelope();
    empty_conversation_id.conversation.id.clear();
    let invalid_payloads = [
        serde_json::to_value(wrong_account).expect("encode wrong-account envelope"),
        serde_json::to_value(wrong_provider).expect("encode wrong-provider envelope"),
        serde_json::to_value(empty_message_id).expect("encode empty-message-id envelope"),
        serde_json::to_value(empty_conversation_id).expect("encode empty-conversation-id envelope"),
        serde_json::json!({
            "channel": { "id": "telegram", "account_id": "telegram:bot-a" },
            "conversation": "not-a-conversation",
            "parts": []
        }),
    ];
    let legacy = tinychannels_bus::ChannelMessage {
        channel: "telegram:bot-a".into(),
        content: "legacy message".into(),
        ..Default::default()
    };
    let mut malformed_relay_with_legacy_fields =
        serde_json::to_value(&legacy).expect("encode legacy message");
    malformed_relay_with_legacy_fields["envelope"] = serde_json::json!("malformed");
    let mut malformed_provenance_with_legacy_fields =
        serde_json::to_value(&legacy).expect("encode legacy message");
    malformed_provenance_with_legacy_fields["delivered_via_authenticated_relay"] =
        serde_json::json!("not-a-boolean");
    let mut malformed_typed_with_legacy_fields =
        serde_json::to_value(&legacy).expect("encode legacy message");
    malformed_typed_with_legacy_fields["parts"] = serde_json::json!("malformed");
    let invalid_payloads = invalid_payloads.into_iter().chain([
        malformed_relay_with_legacy_fields,
        malformed_provenance_with_legacy_fields,
        malformed_typed_with_legacy_fields,
    ]);

    for payload in invalid_payloads {
        assert!(
            callback
                .deliver_inbound("telegram:bot-a".into(), payload)
                .await
                .is_err(),
            "invalid typed or relay input must fail closed"
        );
    }
    assert!(
        matches!(
            received.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ),
        "rejected input must not reach channel dispatch"
    );
}

#[tokio::test]
async fn trusted_relay_provenance_and_legacy_inbound_remain_supported() {
    let (dispatch, mut received) = tokio::sync::mpsc::channel(2);
    let callback = ChannelHostCallbacks::new(
        &crate::core::runtime::DispatchCtx::host_default(),
        Arc::new(crate::config::Config::default()),
        Arc::new(OnceLock::new()),
        Arc::new(OnceLock::from(Arc::new(HashMap::new()))),
        None,
        dispatch,
    );
    let envelope = tinychannels_bus::ChannelInboundEnvelope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some("telegram:bot-a".into()),
        },
        message_id: "message-2".into(),
        conversation: tinychannels_bus::ConversationRef {
            id: "42".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let relay = tinychannels_bus::module_relay::RelayInboundDelivery {
        envelope,
        delivered_via_authenticated_relay: true,
    };
    callback
        .deliver_inbound(
            "telegram".into(),
            serde_json::to_value(relay).expect("encode relay delivery"),
        )
        .await
        .expect("trusted relay delivery remains accepted");
    assert!(received.try_recv().unwrap().authenticated_relay);

    let legacy = tinychannels_bus::ChannelMessage {
        channel: "telegram".into(),
        content: "legacy message".into(),
        ..Default::default()
    };
    callback
        .deliver_inbound(
            "telegram".into(),
            serde_json::to_value(legacy).expect("encode legacy message"),
        )
        .await
        .expect("legacy callback remains supported");
    let legacy_runtime_message = received.try_recv().unwrap();
    assert!(!legacy_runtime_message.authenticated_relay);
    assert!(legacy_runtime_message.inbound_envelope.is_none());
    assert_eq!(legacy_runtime_message.message.content, "legacy message");
}
