use super::*;

#[tokio::test]
async fn native_staged_lease_cleans_dropped_queue_and_preserves_committed_claims() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("actions");
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread_id = "telegram:account-a:conversation-42";
    let workspace = crate::attachments::thread_action_dir(
        &ctx,
        &config,
        &config.workspace_dir,
        Some(thread_id),
    )
    .await
    .unwrap();
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    let repo = test_repo(&config.workspace_dir);

    let pending_id = uuid::Uuid::new_v4().simple().to_string();
    let pending = image(MediaReference {
        opaque_id: Some(pending_id.clone()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    let pending_path = "pending.png";
    let pending_file = workspace.join(pending_path);
    tokio::fs::write(&pending_file, b"pending bytes")
        .await
        .unwrap();
    let mut pending_record = staged_record(pending.clone(), pending_path);
    pending_record.thread_id = thread_id.into();
    insert_record(&repo, &pending_record);
    let lease = NativeStagedLease::new(
        ctx.clone(),
        thread_id.into(),
        config.clone(),
        vec![pending.clone()],
    );
    let queued_clone = lease.clone();
    drop(lease);
    assert!(
        get_staged_record(&repo, &staged_doc_id(&pending_id).unwrap())
            .unwrap()
            .unwrap()
            .claim
            .is_none()
    );
    drop(queued_clone);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let record = get_staged_record(&repo, &staged_doc_id(&pending_id).unwrap())
                .unwrap()
                .unwrap();
            if record.claim.is_some_and(|claim| claim.canceled) && !pending_file.exists() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the final queued payload clone should discard its staged ref");
    assert!(!pending_file.exists());

    let removed_id = uuid::Uuid::new_v4().simple().to_string();
    let removed = image(MediaReference {
        opaque_id: Some(removed_id.clone()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    let removed_path = "removed-from-queue.png";
    let removed_file = workspace.join(removed_path);
    tokio::fs::write(&removed_file, b"removed bytes")
        .await
        .unwrap();
    let mut removed_record = staged_record(removed.clone(), removed_path);
    removed_record.thread_id = thread_id.into();
    insert_record(&repo, &removed_record);
    let queue_lease = NativeStagedLease::new(
        ctx.clone(),
        thread_id.into(),
        config.clone(),
        vec![removed.clone()],
    );
    let queue = tinyagents_harness::run_queue::RunQueue::new();
    let queued = tinyagents_harness::turn_runner::TurnRequest {
        scope: tinyagents_harness::turn_runner::ScopeKey {
            agent_id: "orchestrator".into(),
            profile_id: None,
        },
        thread_id: thread_id.into(),
        request_id: "remove-native-media".into(),
        text: "queued image".into(),
        payload: crate::agent::queued_turn::QueuedPayload {
            native_staged_lease: Some(queue_lease),
            ..Default::default()
        },
        host_authored: false,
        disposition: tinyagents_harness::turn_runner::QueueDisposition::Followup,
        deadline: None,
        idle_timeout: None,
    };
    queue
        .push(tinyagents_harness::run_queue::QueueLane::Followup, queued)
        .await;
    assert_eq!(
        queue
            .remove_where(|request| request.request_id == "remove-native-media")
            .await,
        1
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let record = get_staged_record(&repo, &staged_doc_id(&removed_id).unwrap())
                .unwrap()
                .unwrap();
            if record.claim.is_some_and(|claim| claim.canceled) && !removed_file.exists() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("removing the queued request should discard its staged media");
    assert!(!removed_file.exists());

    let committed_id = uuid::Uuid::new_v4().simple().to_string();
    let committed = image(MediaReference {
        opaque_id: Some(committed_id.clone()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    let committed_path = "committed.png";
    let committed_file = workspace.join(committed_path);
    tokio::fs::write(&committed_file, b"committed bytes")
        .await
        .unwrap();
    let mut committed_record = staged_record(committed.clone(), committed_path);
    committed_record.thread_id = thread_id.into();
    insert_record(&repo, &committed_record);
    let durable_claim = claim("user:request", committed.clone(), false);
    claim_staged_record(
        &repo,
        &durable_claim.tenant,
        thread_id,
        &staged_doc_id(&committed_id).unwrap(),
        &committed,
        &durable_claim,
    )
    .unwrap();
    let committed_lease =
        NativeStagedLease::new(ctx, thread_id.into(), config, vec![committed.clone()]);
    let queue_clone = committed_lease.clone();
    committed_lease.mark_committed();
    drop(committed_lease);
    drop(queue_clone);
    tokio::task::yield_now().await;
    let stored = get_staged_record(&repo, &staged_doc_id(&committed_id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored.claim, Some(durable_claim));
    assert_eq!(
        tokio::fs::read(committed_file).await.unwrap(),
        b"committed bytes"
    );
}

#[tokio::test]
async fn native_staged_lease_retries_cleanup_for_its_canceled_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("actions");
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread_id = "telegram:account-a:conversation-canceled";
    let workspace = crate::attachments::thread_action_dir(
        &ctx,
        &config,
        &config.workspace_dir,
        Some(thread_id),
    )
    .await
    .unwrap();
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    let repo = test_repo(&config.workspace_dir);
    let opaque_id = uuid::Uuid::new_v4().simple().to_string();
    let part = image(MediaReference {
        opaque_id: Some(opaque_id.clone()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    let path = "canceled-before-retry.png";
    let file = workspace.join(path);
    tokio::fs::write(&file, b"canceled cleanup bytes")
        .await
        .unwrap();
    let mut record = staged_record(part.clone(), path);
    record.thread_id = thread_id.into();
    record.claim = Some(StagedAttachmentClaim {
        tenant: crate::core::runtime::Tenant::default(),
        thread_id: thread_id.into(),
        message_id: String::new(),
        part: part.clone(),
        canceled: true,
    });
    insert_record(&repo, &record);
    assert!(
        read_staged_entries(&ctx, thread_id, std::slice::from_ref(&part), &config)
            .await
            .is_err(),
        "a canceled reference must remain unavailable to ordinary reads"
    );

    drop(NativeStagedLease::new(
        ctx,
        thread_id.into(),
        config,
        vec![part],
    ));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while file.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the lease should retry cleanup after its exact canceled tombstone");
}

#[tokio::test]
async fn canceling_native_media_resolution_cleans_the_first_staged_part() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("actions");
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread_id = "telegram:account-a:conversation-42";
    let (staged_tx, staged_rx) = tokio::sync::oneshot::channel();
    let task_ctx = ctx.clone();
    let task_config = config.clone();
    let task = tokio::spawn(async move {
        let lease = NativeStagedLease::new(
            task_ctx.clone(),
            thread_id.into(),
            task_config.clone(),
            Vec::new(),
        );
        let source = image(MediaReference {
            content_type: Some("image/png".into()),
            kind: tinychannels_bus::MediaKind::Image,
            ..Default::default()
        });
        let staged = crate::threads::attachments::stage_native_channel_part(
            &task_ctx,
            thread_id,
            &task_config,
            &source,
            "first-native.png",
            "image/png",
            b"cancellation lifecycle fixture",
            &lease,
        )
        .await
        .unwrap();
        staged_tx.send(staged).unwrap();
        std::future::pending::<()>().await;
    });
    let staged = staged_rx.await.unwrap();
    let opaque_id = opaque_media(&staged)
        .and_then(|media| media.opaque_id.as_deref())
        .unwrap()
        .to_owned();
    let workspace = crate::attachments::thread_action_dir(
        &ctx,
        &config,
        &config.workspace_dir,
        Some(thread_id),
    )
    .await
    .unwrap();
    let repo = test_repo(&config.workspace_dir);
    let record = get_staged_record(&repo, &staged_doc_id(&opaque_id).unwrap())
        .unwrap()
        .unwrap();
    let staged_file = workspace.join(record.attachment.path);
    assert!(
        staged_file.exists(),
        "the first staged part is durable before cancellation"
    );

    task.abort();
    let _ = task.await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let record = get_staged_record(&repo, &staged_doc_id(&opaque_id).unwrap())
                .unwrap()
                .unwrap();
            if record.claim.is_some_and(|claim| claim.canceled) && !staged_file.exists() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("canceling resolution must discard already-staged media");
}
