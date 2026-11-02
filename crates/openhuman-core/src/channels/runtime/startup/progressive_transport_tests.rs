use super::*;

#[test]
fn forbidden_edit_error_is_preserved_instead_of_reported_as_unsupported() {
    let outcome = tinychannels_bus::channel::ChannelSendOutcome {
        receipt: Default::default(),
        error: Some(tinychannels_bus::channel::ChannelSendError {
            kind: tinychannels_bus::channel::SendErrorKind::Forbidden,
            message: "edit denied by account policy".into(),
            ..Default::default()
        }),
    };

    let tinychannels_bus::module_progressive::ProgressiveTransportResult::Outcome(mapped) =
        map_edit_outcome(outcome)
    else {
        panic!("a forbidden edit error must remain an outcome");
    };
    let error = mapped.error.expect("the denial details must be preserved");
    assert_eq!(
        error.kind,
        tinychannels_bus::channel::SendErrorKind::Forbidden
    );
    assert_eq!(error.message, "edit denied by account policy");
    assert!(!error.unsupported);
}

#[test]
fn module_callback_error_does_not_expose_provider_details() {
    let error = sanitize_module_error(
        "edit",
        "request failed for https://api.invalid/bot?token=SECRET",
    );

    assert!(error.to_string().contains("channel edit callback failed"));
    assert!(!error.to_string().contains("SECRET"));
    assert!(!error.to_string().contains("api.invalid"));
}

#[test]
fn receipt_scope_requires_exact_channel_and_full_conversation_identity() {
    use tinychannels_bus::channel::{types::ConversationKind, MessageReceipt};
    use tinychannels_bus::{ChannelRef, ConversationRef};

    let channel = ChannelRef {
        id: "telegram".into(),
        account_id: Some("telegram:bot-a".into()),
    };
    let conversation = ConversationRef {
        kind: ConversationKind::Group,
        id: "chat-42".into(),
        scope_id: Some("tenant-1".into()),
        parent_id: Some("group-7".into()),
        thread_id: Some("thread-9".into()),
        topic_id: Some("topic-5".into()),
    };
    let mut receipt = MessageReceipt {
        channel: Some(channel.clone()),
        conversation: Some(conversation.clone()),
        ..Default::default()
    };
    assert!(receipt_matches_scope(
        &receipt,
        Some(&channel),
        Some(&conversation)
    ));
    assert!(!receipt_matches_scope(&receipt, Some(&channel), None));
    assert!(!receipt_matches_scope(
        &MessageReceipt::default(),
        Some(&channel),
        Some(&conversation)
    ));

    let mut different_account = channel.clone();
    different_account.account_id = Some("telegram:bot-b".into());
    assert!(!receipt_matches_scope(
        &receipt,
        Some(&different_account),
        Some(&conversation)
    ));

    let mut different_kind = conversation.clone();
    different_kind.kind = ConversationKind::Dm;
    assert!(!receipt_matches_scope(
        &receipt,
        Some(&channel),
        Some(&different_kind)
    ));
    receipt.conversation = Some(ConversationRef {
        thread_id: Some("different-thread".into()),
        ..conversation.clone()
    });
    assert!(!receipt_matches_scope(
        &receipt,
        Some(&channel),
        Some(&conversation)
    ));
}
