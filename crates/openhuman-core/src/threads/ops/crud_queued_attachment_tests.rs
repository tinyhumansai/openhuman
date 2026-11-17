use crate::threads::ops::persist_queued_user_message;
use crate::threads::store as conversations;
use base64::Engine as _;

async fn setup() -> (tempfile::TempDir, crate::config::Config) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = temp.path().join("internal");
    config.action_dir = temp.path().join("acting");
    conversations::blocking::ensure_thread(
        crate::core::runtime::DispatchCtx::ambient(),
        config.workspace_dir.clone(),
        conversations::CreateConversationThread {
            id: "uploads".into(),
            title: "Uploads".into(),
            created_at: "2026-10-04T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    (temp, config)
}

#[tokio::test]
async fn queued_user_append_reuses_exact_request_row_and_rejects_conflicts() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use tinychannels_bus::{ContentPart, MediaReference};

    let (_temp, config) = setup().await;
    let ctx = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        config.clone(),
    ));
    let staged = crate::threads::attachments::stage_queued_parts(
        &ctx,
        "uploads",
        &[ContentPart::Image {
            media: MediaReference {
                url: Some(format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(include_bytes!(
                        "../../../../../tests/fixtures/images/handoff-hero.png"
                    ))
                )),
                ..Default::default()
            },
            caption: None,
            alt_text: None,
        }],
    )
    .await
    .unwrap();
    let first = persist_queued_user_message(&ctx, "uploads", "request-1", "", &staged)
        .await
        .unwrap();
    assert_eq!(first.id, "user:request-1");
    assert_eq!(first.sender, "user");
    assert!(first.content.is_empty());
    assert_eq!(first.attachments, staged);
    let resolved = crate::threads::attachments::resolve_persisted_parts(
        &ctx,
        "uploads",
        &first.id,
        &first.attachments,
    )
    .await
    .expect("queued refs are bound to the persisted row");
    let path = match &resolved[0] {
        ContentPart::Image { media, .. } => media.path.as_deref().expect("resolved path"),
        other => panic!("expected image attachment, got {other:?}"),
    };
    assert_eq!(
        tokio::fs::read(config.action_dir.join(path)).await.unwrap(),
        include_bytes!("../../../../../tests/fixtures/images/handoff-hero.png")
    );

    let retry = persist_queued_user_message(&ctx, "uploads", "request-1", "", &staged)
        .await
        .unwrap();
    assert_eq!(retry.id, first.id);

    let conflicting_retry =
        persist_queued_user_message(&ctx, "uploads", "request-1", "different content", &staged)
            .await;
    assert!(conflicting_retry.is_err());

    let rows =
        conversations::blocking::get_messages(ctx, config.workspace_dir.clone(), "uploads".into())
            .await
            .unwrap();
    assert_eq!(rows.iter().filter(|row| row.id == first.id).count(), 1);
}

#[tokio::test]
async fn queued_text_only_user_message_is_persisted_idempotently() {
    let (_temp, config) = setup().await;
    let ctx = crate::core::runtime::DispatchCtx::new(
        crate::core::runtime::CoreContext::for_test_with_config(
            crate::core::runtime::DomainSet::full(),
            config.clone(),
        ),
    );

    let first = persist_queued_user_message(&ctx, "uploads", "text-only", "follow-up", &[])
        .await
        .unwrap();
    let retry = persist_queued_user_message(&ctx, "uploads", "text-only", "follow-up", &[])
        .await
        .unwrap();

    assert_eq!(first.id, "user:text-only");
    assert_eq!(retry.id, first.id);
    assert_eq!(first.sender, "user");
    assert_eq!(first.content, "follow-up");
    assert!(first.attachments.is_empty());
    let rows = conversations::get_messages(&ctx, config.workspace_dir, "uploads").unwrap();
    assert_eq!(
        rows.iter().filter(|row| row.id == "user:text-only").count(),
        1
    );
}

#[tokio::test]
async fn concurrent_queued_attachment_transfer_has_one_exact_owner() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use tinychannels_bus::{ContentPart, MediaReference};

    let (_temp, config) = setup().await;
    let ctx = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        config.clone(),
    ));
    let image = || ContentPart::Image {
        media: MediaReference {
            url: Some(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(include_bytes!(
                    "../../../../../tests/fixtures/images/handoff-hero.png"
                ))
            )),
            ..Default::default()
        },
        caption: None,
        alt_text: None,
    };

    let staged_conflict =
        crate::threads::attachments::stage_queued_parts(&ctx, "uploads", &[image()])
            .await
            .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let left_ctx = ctx.clone();
    let left_refs = staged_conflict.clone();
    let left_barrier = barrier.clone();
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        persist_queued_user_message(&left_ctx, "uploads", "same", "left", &left_refs).await
    });
    let right_ctx = ctx.clone();
    let right_refs = staged_conflict.clone();
    let right = tokio::spawn(async move {
        barrier.wait().await;
        persist_queued_user_message(&right_ctx, "uploads", "same", "right", &right_refs).await
    });
    let results = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let rows = conversations::get_messages(&ctx, config.workspace_dir.clone(), "uploads").unwrap();
    assert_eq!(rows.iter().filter(|row| row.id == "user:same").count(), 1);

    let staged_retry = crate::threads::attachments::stage_queued_parts(&ctx, "uploads", &[image()])
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let left_ctx = ctx.clone();
    let left_refs = staged_retry.clone();
    let left_barrier = barrier.clone();
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        persist_queued_user_message(&left_ctx, "uploads", "same-retry", "same", &left_refs).await
    });
    let right_ctx = ctx.clone();
    let right_refs = staged_retry.clone();
    let right = tokio::spawn(async move {
        barrier.wait().await;
        persist_queued_user_message(&right_ctx, "uploads", "same-retry", "same", &right_refs).await
    });
    let results = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 2);

    let staged_single_owner =
        crate::threads::attachments::stage_queued_parts(&ctx, "uploads", &[image()])
            .await
            .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let left_ctx = ctx.clone();
    let left_refs = staged_single_owner.clone();
    let left_barrier = barrier.clone();
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        persist_queued_user_message(&left_ctx, "uploads", "owner-a", "", &left_refs).await
    });
    let right_ctx = ctx.clone();
    let right_refs = staged_single_owner.clone();
    let right = tokio::spawn(async move {
        barrier.wait().await;
        persist_queued_user_message(&right_ctx, "uploads", "owner-b", "", &right_refs).await
    });
    let results = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
}
