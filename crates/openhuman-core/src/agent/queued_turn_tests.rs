use super::*;
use crate::agent::host_agents::HostAgent;
use crate::core::runtime::{CoreContext, DomainSet};

fn trusted_turn(session_key: &str) -> Arc<super::super::turn_host::TrustedAgentTurn> {
    let definition =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
            .get("orchestrator")
            .cloned()
            .expect("built-in orchestrator definition");
    Arc::new(super::super::turn_host::TrustedAgentTurn {
        host_agent: HostAgent {
            definition,
            config: crate::config::Config::default(),
            host_tools: None,
            context: CoreContext::for_test(DomainSet::full(), None),
            hooks: Default::default(),
        },
        channel: "telegram".into(),
        session_key: session_key.into(),
        event_session_id: "event-scope".into(),
        origin: crate::security::turn_origin::AgentTurnOrigin::ExternalChannel {
            channel: "telegram".into(),
            sender: Some("sender-a".into()),
            sender_name: Some("Alice".into()),
            reply_target: "conversation-a".into(),
            message_id: "message-a".into(),
            history_key: Some("legacy-history-a".into()),
            channel_session: None,
        },
        approval: crate::security::approval::ApprovalTurnContext::chat(
            crate::security::approval::ApprovalChatContext {
                thread_id: "conversation-a".into(),
                client_id: "telegram".into(),
                request_id: Some("message-a".into()),
            },
        ),
        relay_thread: Some("relay-a".into()),
        imported_history: vec![tinyagents_session::transcript::TranscriptMessage::user(
            "trusted imported row",
        )],
        #[cfg(feature = "channels")]
        legacy_history: None,
        #[cfg(feature = "channels")]
        native_target: None,
    })
}

fn marker(name: &str) -> String {
    crate::attachments::Attachment {
        path: "uploads/thread/id/file.zip".into(),
        name: name.into(),
        mime: "application/zip".into(),
        size_bytes: 3,
    }
    .marker()
}

#[test]
fn raw_and_staged_names_match_without_decoding_payloads() {
    let raw = "[FILE:data:application/zip;name=Report+%CE%B1.zip;base64,opaque-invalid-payload]";
    assert_eq!(text_preview(raw), "Report α.zip");
    assert_eq!(text_preview(raw), text_preview(&marker("Report α.zip")));
    assert_eq!(text_preview("[IMAGE:/local/file.png]"), "attachment");
    assert_eq!(
        text_preview("[FILE:data:audio/mpeg;base64,secret]"),
        "attachment"
    );
}

#[test]
fn captions_match_after_staging_and_preserve_paragraphs() {
    let raw =
        "  Check   this [FILE:data:application/zip;name=a.zip;base64,secret]\n\nnext paragraph  ";
    let staged = format!("  Check   this {}\n\nnext paragraph  ", marker("a.zip"));
    assert_eq!(text_preview(raw), "Check this \n\nnext paragraph");
    assert_eq!(text_preview(raw), text_preview(&staged));
}

#[test]
fn mixed_markers_keep_source_order() {
    let mixed = format!(
        "[IMAGE:data:image/png;name=first.png;base64,secret] {} [FILE:data:audio/mpeg;name=third.mp3;base64,secret]",
        marker("second α.zip")
    );
    assert_eq!(text_preview(&mixed), "first.png, second α.zip, third.mp3");
}

#[test]
fn invalid_durable_reference_stays_literal() {
    let invalid = crate::attachments::Attachment {
        path: "../escape.zip".into(),
        name: "escape.zip".into(),
        mime: "application/zip".into(),
        size_bytes: 3,
    }
    .marker();
    assert_eq!(
        text_preview(&invalid),
        crate::core::events::clip_to_chars(&invalid, 80)
    );
    assert_eq!(text_preview("[ATTACHMENT:invalid]"), "[ATTACHMENT:invalid]");
}

#[test]
fn clips_unicode_codepoints_for_captions_and_names() {
    let long = "🦀".repeat(81);
    let expected = format!("{}…", "🦀".repeat(80));
    assert_eq!(text_preview(&long), expected);
    assert_eq!(text_preview(&marker(&long)), expected);
    assert_eq!(text_preview(&"α".repeat(80)), "α".repeat(80));
}

#[test]
fn requeued_placeholder_adopts_trusted_agent_and_channel_identity() {
    let trusted_turn = trusted_turn("tenant-a:telegram:account-a:conversation-a");
    let running = QueuedPayload {
        client_id: "client-a".into(),
        trusted_turn: Some(Arc::clone(&trusted_turn)),
        ..Default::default()
    };
    let mut requeued = QueuedPayload::requeued("thread-a", 7);

    RequeuePayload::adopt(&mut requeued, &running);

    let adopted = requeued
        .trusted_turn
        .expect("trusted execution is preserved");
    assert!(Arc::ptr_eq(&adopted, &trusted_turn));
    assert_eq!(adopted.host_agent.definition.id, "orchestrator");
    assert_eq!(
        adopted.session_key,
        "tenant-a:telegram:account-a:conversation-a"
    );
    assert_eq!(adopted.channel, "telegram");
    assert!(matches!(
        &adopted.origin,
        crate::security::turn_origin::AgentTurnOrigin::ExternalChannel {
            sender: Some(sender), ..
        } if sender == "sender-a"
    ));
    assert_eq!(adopted.imported_history.len(), 1);
}

#[test]
fn a_populated_requeue_keeps_its_own_trusted_execution() {
    let running = QueuedPayload {
        trusted_turn: Some(trusted_turn("tenant-a:telegram:account-a:one")),
        ..Default::default()
    };
    let own_turn = trusted_turn("tenant-b:telegram:account-b:two");
    let mut queued = QueuedPayload {
        client_id: "client-b".into(),
        trusted_turn: Some(Arc::clone(&own_turn)),
        ..Default::default()
    };

    RequeuePayload::adopt(&mut queued, &running);

    assert!(Arc::ptr_eq(
        queued.trusted_turn.as_ref().expect("own authority remains"),
        &own_turn
    ));
}

#[test]
fn requeue_adopts_context_but_never_predecessor_message_or_attachment_state() {
    let running = QueuedPayload {
        client_id: "client-a".into(),
        trusted_turn: Some(trusted_turn("tenant-a:telegram:account-a:one")),
        deferred_user_message: true,
        pending_attachments: Some(PendingAttachmentInput {
            parts: vec![tinychannels_bus::ContentPart::Image {
                media: tinychannels_bus::MediaReference::default(),
                caption: None,
                alt_text: None,
            }],
        }),
        stored_attachments: Some(StoredAttachmentInput {
            persisted_message_id: "user:prior-image".into(),
            parts: vec![],
        }),
        native_message_input: Some(tinyinference_llm::message::Message::user(
            "prior request typed content",
        )),
        ..Default::default()
    };
    let mut next = QueuedPayload {
        client_id: "client-b".into(),
        ..Default::default()
    };

    RequeuePayload::adopt(&mut next, &running);

    assert!(
        next.trusted_turn.is_some(),
        "trusted channel context is inherited"
    );
    assert!(!next.deferred_user_message);
    assert!(next.pending_attachments.is_none());
    assert!(next.stored_attachments.is_none());
    assert!(next.native_message_input.is_none());
}
