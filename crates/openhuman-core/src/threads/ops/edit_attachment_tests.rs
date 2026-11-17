use super::*;
use base64::Engine as _;

use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
use crate::threads::store::{self, CreateConversationThread};
use crate::threads::{AppendConversationMessageRequest, ConversationMessageRecord};
use tinychannels_bus::{ContentPart, MediaReference};

#[tokio::test]
async fn invalid_edit_attachment_leaves_the_original_message_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = temp.path().join("workspace");
    config.action_dir = temp.path().join("acting");
    let ctx = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        config.clone(),
    ));
    store::blocking::ensure_thread(
        ctx.clone(),
        config.workspace_dir.clone(),
        CreateConversationThread {
            id: "edit-attachments".into(),
            title: "Edit attachments".into(),
            created_at: "2026-10-20T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    crate::threads::ops::message_append(
        &ctx,
        AppendConversationMessageRequest {
            thread_id: "edit-attachments".into(),
            message: ConversationMessageRecord {
                id: "user-original".into(),
                content: "original prompt".into(),
                attachments: Vec::new(),
                message_type: "text".into(),
                extra_metadata: serde_json::json!({}),
                sender: "user".into(),
                created_at: "2026-10-20T00:00:01Z".into(),
            },
            attachments: Vec::new(),
        },
    )
    .await
    .unwrap();

    let result = edit_message(
        &ctx,
        EditMessageRequest {
            thread_id: "edit-attachments".into(),
            message_id: "user-original".into(),
            content: "replacement prompt".into(),
            attachments: vec![ContentPart::Image {
                media: MediaReference {
                    path: Some("/etc/passwd".into()),
                    ..Default::default()
                },
                caption: None,
                alt_text: None,
            }],
            client_id: Some("test-client".into()),
        },
    )
    .await;
    assert!(result.is_err());

    let rows = store::blocking::get_messages(ctx, config.workspace_dir, "edit-attachments".into())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "user-original");
    assert_eq!(rows[0].content, "original prompt");
}

#[tokio::test]
async fn editing_an_image_row_restages_only_its_exact_opaque_reference() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = temp.path().join("workspace");
    config.action_dir = temp.path().join("acting");
    let ctx = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        config.clone(),
    ));
    store::blocking::ensure_thread(
        ctx.clone(),
        config.workspace_dir.clone(),
        CreateConversationThread {
            id: "edit-image-source".into(),
            title: "Edit image source".into(),
            created_at: "2026-10-20T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    let source = crate::threads::ops::message_append(
        &ctx,
        AppendConversationMessageRequest {
            thread_id: "edit-image-source".into(),
            message: ConversationMessageRecord {
                id: "user-image-source".into(),
                content: "describe this".into(),
                attachments: Vec::new(),
                message_type: "text".into(),
                extra_metadata: serde_json::json!({}),
                sender: "user".into(),
                created_at: "2026-10-20T00:00:01Z".into(),
            },
            attachments: vec![ContentPart::Image {
                media: MediaReference {
                    url: Some(format!(
                        "data:image/png;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(include_bytes!(
                            "../../../../../tests/fixtures/images/handoff-hero.png"
                        ))
                    )),
                    content_type: Some("image/png".into()),
                    kind: tinychannels_bus::MediaKind::Image,
                    ..Default::default()
                },
                caption: None,
                alt_text: None,
            }],
        },
    )
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert_eq!(source.attachments.len(), 1);
    let original_ref = source.attachments[0].clone();

    let replacement_refs = crate::threads::attachments::restage_persisted_parts(
        &ctx,
        "edit-image-source",
        &source.id,
        std::slice::from_ref(&original_ref),
    )
    .await
    .unwrap();
    assert_eq!(replacement_refs.len(), 1);
    assert_ne!(replacement_refs[0], original_ref);
    crate::threads::attachments::validate_opaque_parts(&replacement_refs).unwrap();

    let other_row = crate::threads::attachments::restage_persisted_parts(
        &ctx,
        "edit-image-source",
        "user-someone-elses-row",
        &[original_ref],
    )
    .await;
    assert!(other_row.is_err());
    crate::threads::attachments::discard_staged_parts(&ctx, "edit-image-source", &replacement_refs)
        .await
        .unwrap();
}

#[tokio::test]
async fn edit_rpc_rejects_an_attachment_ref_owned_by_another_row_before_truncation() {
    let _serial = crate::web_chat::RUN_CHAT_TASK_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = temp.path().join("workspace");
    config.action_dir = temp.path().join("acting");
    let ctx = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        config.clone(),
    ));
    store::blocking::ensure_thread(
        ctx.clone(),
        config.workspace_dir.clone(),
        CreateConversationThread {
            id: "edit-image-scope".into(),
            title: "Edit image scope".into(),
            created_at: "2026-10-20T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    let image = || ContentPart::Image {
        media: MediaReference {
            url: Some(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(include_bytes!(
                    "../../../../../tests/fixtures/images/handoff-hero.png"
                ))
            )),
            content_type: Some("image/png".into()),
            kind: tinychannels_bus::MediaKind::Image,
            ..Default::default()
        },
        caption: None,
        alt_text: None,
    };
    let append_image_row = |id: &str, content: &str| {
        let ctx = ctx.clone();
        let id = id.to_owned();
        let content = content.to_owned();
        let image = image();
        async move {
            crate::threads::ops::message_append(
                &ctx,
                AppendConversationMessageRequest {
                    thread_id: "edit-image-scope".into(),
                    message: ConversationMessageRecord {
                        id,
                        content,
                        attachments: Vec::new(),
                        message_type: "text".into(),
                        extra_metadata: serde_json::json!({}),
                        sender: "user".into(),
                        created_at: "2026-10-20T00:00:01Z".into(),
                    },
                    attachments: vec![image],
                },
            )
            .await
            .unwrap()
            .value
            .data
            .unwrap()
        }
    };
    let source = append_image_row("user-image-source", "source row").await;
    let other = append_image_row("user-image-other", "other row").await;

    let block = crate::web_chat::TestRunChatTaskBlock {
        started: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        dropped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        release: std::sync::Arc::new(tokio::sync::Notify::new()),
        max_park_for: std::time::Duration::from_secs(30),
        succeed_in: None,
    };
    crate::web_chat::set_test_run_chat_task_block(Some(block.clone())).await;
    crate::web_chat::start_chat(
        &ctx,
        "active-edit-client",
        "edit-image-scope",
        "keep this turn alive",
        None,
        None,
        None,
        None,
        crate::web_chat::ChatRequestMetadata::default(),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !block.started.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let result = edit_message(
        &ctx,
        EditMessageRequest {
            thread_id: "edit-image-scope".into(),
            message_id: source.id.clone(),
            content: "replacement text".into(),
            attachments: other.attachments.clone(),
            client_id: Some("test-client".into()),
        },
    )
    .await;
    let rejected = result.is_err();
    let active_turn_was_preserved = !block.dropped.load(std::sync::atomic::Ordering::SeqCst);

    let _ =
        crate::web_chat::cancel_chat_scoped(&ctx, "active-edit-client", "edit-image-scope", None)
            .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !block.dropped.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    crate::web_chat::set_test_run_chat_task_block(None).await;

    assert!(rejected, "foreign opaque refs must be rejected");
    assert!(
        active_turn_was_preserved,
        "foreign attachment rejection must leave the active turn alone"
    );

    let rows = store::blocking::get_messages(
        ctx.clone(),
        config.workspace_dir.clone(),
        "edit-image-scope".into(),
    )
    .await
    .unwrap();
    let source_after = rows.iter().find(|row| row.id == source.id).unwrap();
    assert_eq!(source_after.content, "source row");
    assert_eq!(
        crate::threads::attachments::public_refs(&source_after.extra_metadata),
        source.attachments
    );
    let other_after = rows.iter().find(|row| row.id == other.id).unwrap();
    assert_eq!(other_after.content, "other row");
    assert_eq!(
        crate::threads::attachments::public_refs(&other_after.extra_metadata),
        other.attachments
    );
}
