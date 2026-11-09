use super::staged::{STAGED_COLLECTION, STAGED_DOMAIN};
use super::*;

fn image(media: MediaReference) -> ContentPart {
    ContentPart::Image {
        media,
        caption: None,
        alt_text: None,
    }
}

#[test]
fn persisted_refs_require_unique_opaque_ids_without_source_authority() {
    let valid = image(MediaReference {
        opaque_id: Some("opaque-1".into()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    assert!(validate_opaque_parts(std::slice::from_ref(&valid)).is_ok());

    for media in [
        MediaReference {
            opaque_id: Some("opaque-1".into()),
            path: Some("../../etc/passwd".into()),
            ..Default::default()
        },
        MediaReference {
            opaque_id: Some("opaque-1".into()),
            url: Some("data:image/png;base64,AAAA".into()),
            ..Default::default()
        },
        MediaReference::default(),
    ] {
        assert!(validate_opaque_parts(&[image(media)]).is_err());
    }
    assert!(validate_opaque_parts(&[valid.clone(), valid]).is_err());
}

#[test]
fn public_projection_hides_paths_but_keeps_opaque_attachment_refs() {
    let part = image(MediaReference {
        opaque_id: Some("opaque-2".into()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    let mut metadata = json!({"keep": true});
    let public = save(
        &mut metadata,
        vec![(part.clone(), "uploads/thread/a.png".into())],
    );
    assert_eq!(public, vec![part]);
    assert!(metadata[STORAGE_KEY][0]["path"]
        .as_str()
        .unwrap()
        .contains("uploads/"));

    let projected = public_refs(&metadata);
    assert_eq!(projected, public);
    strip_private_metadata(&mut metadata);
    assert_eq!(metadata, json!({"keep": true}));
}

#[test]
fn staged_image_reference_preserves_uploaded_file_name() {
    let part = image(MediaReference {
        file_name: Some("queued-roundtrip.png".into()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    let staged = Attachment {
        path: "uploads/thread/attachment".into(),
        name: "attachment".into(),
        mime: "image/png".into(),
        size_bytes: 68,
    };

    let result = from_staged_parts(&[part], &[staged]).unwrap();
    let ContentPart::Image { media, .. } = &result[0].0 else {
        panic!("staged image changed kind");
    };
    assert_eq!(media.file_name.as_deref(), Some("queued-roundtrip.png"));
}

fn claim(message_id: &str, part: ContentPart, canceled: bool) -> StagedAttachmentClaim {
    StagedAttachmentClaim {
        tenant: crate::core::runtime::Tenant::default(),
        thread_id: "thread".into(),
        message_id: message_id.into(),
        part,
        canceled,
    }
}

fn test_repo(workspace: &std::path::Path) -> Repo {
    let backend = crate::storage::local::backend_for(workspace).unwrap();
    Repo::on(
        &backend,
        &crate::storage::Scope::local(),
        STAGED_DOMAIN,
        staged_collections,
    )
    .unwrap()
}

fn staged_record(part: ContentPart, path: &str) -> StagedAttachment {
    StagedAttachment {
        tenant: crate::core::runtime::Tenant::default(),
        thread_id: "thread".into(),
        attachment: StoredAttachment {
            part,
            path: path.into(),
        },
        expires_at: chrono::Utc::now().timestamp() + 60,
        claim: None,
    }
}

fn insert_record(repo: &Repo, record: &StagedAttachment) {
    let id = staged_doc_id(
        opaque_media(&record.attachment.part)
            .and_then(|media| media.opaque_id.as_deref())
            .unwrap(),
    )
    .unwrap();
    let body = serde_json::to_value(record).unwrap();
    repo.run(|docs| async move {
        docs.put(STAGED_COLLECTION, &id, body, Precondition::Absent)
            .await?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn staged_claim_and_cancellation_have_one_atomic_document_winner() {
    let dir = tempfile::tempdir().unwrap();
    let repo = test_repo(dir.path());
    let part = image(MediaReference {
        opaque_id: Some(uuid::Uuid::new_v4().simple().to_string()),
        ..Default::default()
    });
    let record = staged_record(part.clone(), "media.png");
    insert_record(&repo, &record);
    let id = staged_doc_id(opaque_media(&part).unwrap().opaque_id.as_deref().unwrap()).unwrap();
    let bound = claim("user:req", part.clone(), false);
    let canceled = claim("", part.clone(), true);
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let claim_task = {
        let barrier = barrier.clone();
        let repo = repo.clone();
        let bound = bound.clone();
        let part = part.clone();
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            claim_staged_record(&repo, &bound.tenant, "thread", &id, &part, &bound)
        })
    };
    let cancel_task = {
        let barrier = barrier.clone();
        let repo = repo.clone();
        let canceled = canceled.clone();
        let part = part.clone();
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            cancel_staged_record(&repo, &canceled.tenant, "thread", &id, &part, &canceled)
        })
    };
    barrier.wait().await;
    let claim_result = claim_task.await.unwrap();
    let cancel_result = cancel_task.await.unwrap();
    assert_ne!(claim_result.is_ok(), cancel_result.is_ok());
    let stored = get_staged_record(&repo, &id).unwrap().unwrap();
    let winner = stored.claim.unwrap();
    if winner.canceled {
        assert!(claim_staged_record(&repo, &bound.tenant, "thread", &id, &part, &bound).is_err());
    } else {
        assert!(
            !cancel_staged_record(&repo, &canceled.tenant, "thread", &id, &part, &canceled)
                .unwrap()
        );
        assert!(!claim_staged_record(&repo, &bound.tenant, "thread", &id, &part, &bound).unwrap());
    }
}

#[tokio::test]
async fn native_attachment_refs_are_bound_to_profile_agent_account_and_turn() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let config = crate::config::Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        ..Default::default()
    };
    let make_dispatch = |profile: &str, agent: &str| {
        let root = CoreContext::for_test(DomainSet::none(), Some(workspace.clone()));
        let scoped = root.derive_with(
            ContextOverlay::new(config.clone(), DomainSet::full(), Default::default())
                .profile(profile)
                .session_agent(agent),
        );
        DispatchCtx::new(scoped)
    };
    let alice = make_dispatch("profile-a", "agent-a");
    let other_profile = make_dispatch("profile-b", "agent-a");
    let other_agent = make_dispatch("profile-a", "agent-b");
    let part = image(MediaReference {
        opaque_id: Some(uuid::Uuid::new_v4().simple().to_string()),
        content_type: Some("image/png".into()),
        size_bytes: Some(4),
        ..Default::default()
    });
    let thread = "telegram:account-a:conversation-42";
    let record = StagedAttachment {
        tenant: crate::core::runtime::Tenant {
            profile: Some("profile-a".into()),
            agent: Some("agent-a".into()),
        },
        thread_id: thread.into(),
        attachment: StoredAttachment {
            part: part.clone(),
            path: "media.png".into(),
        },
        expires_at: chrono::Utc::now().timestamp() + 60,
        claim: None,
    };
    let repo = staged_repo(&alice, &workspace).unwrap();
    insert_record(&repo, &record);
    let opaque_id = opaque_media(&part)
        .unwrap()
        .opaque_id
        .as_deref()
        .unwrap()
        .to_owned();

    for dispatch in [&other_profile, &other_agent] {
        assert!(read_authorized_native_attachment(
            dispatch,
            &config,
            thread,
            &opaque_id,
            std::slice::from_ref(&part),
            false,
        )
        .await
        .is_err());
    }
    assert!(
        read_authorized_native_attachment(
            &alice,
            &config,
            "telegram:account-b:conversation-42",
            &opaque_id,
            std::slice::from_ref(&part),
            false,
        )
        .await
        .is_err(),
        "the same conversation under another configured account is isolated"
    );

    let forged_part = match part.clone() {
        ContentPart::Image {
            media, alt_text, ..
        } => ContentPart::Image {
            media,
            caption: Some("forged turn".into()),
            alt_text,
        },
        _ => unreachable!(),
    };
    assert!(
        read_authorized_native_attachment(
            &alice,
            &config,
            thread,
            &opaque_id,
            &[forged_part],
            false,
        )
        .await
        .is_err(),
        "opaque IDs alone do not authorize a modified part"
    );
    assert!(
        read_authorized_native_attachment(&alice, &config, thread, &opaque_id, &[], false)
            .await
            .is_err(),
        "an unclaimed reference from another turn is not authorized"
    );
    assert!(
        read_authorized_native_attachment(&alice, &config, thread, &opaque_id, &[], true)
            .await
            .is_err(),
        "history access still requires a committed host claim"
    );
}

#[tokio::test]
async fn native_staged_file_reader_enforces_limit_and_rejects_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("small.txt"), b"small").unwrap();
    std::fs::write(root.join("large.txt"), b"larger than bound").unwrap();

    assert_eq!(
        super::staged::read_staged_file_bounded(&root.join("small.txt"), &root, 16)
            .await
            .unwrap(),
        b"small"
    );
    assert!(
        super::staged::read_staged_file_bounded(&root.join("large.txt"), &root, 4)
            .await
            .unwrap_err()
            .contains("configured byte limit")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        std::fs::write(&outside, b"outside workspace").unwrap();
        symlink(&outside, root.join("swapped.txt")).unwrap();
        assert!(
            super::staged::read_staged_file_bounded(&root.join("swapped.txt"), &root, 128,)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn multipart_claim_failure_keeps_prior_claim_owned_by_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("actions");
    let repo = test_repo(&config.workspace_dir);
    let first_id = uuid::Uuid::new_v4().simple().to_string();
    let second_id = uuid::Uuid::new_v4().simple().to_string();
    let first = image(MediaReference {
        opaque_id: Some(first_id.clone()),
        ..Default::default()
    });
    let second = image(MediaReference {
        opaque_id: Some(second_id.clone()),
        ..Default::default()
    });
    insert_record(&repo, &staged_record(first.clone(), "first.png"));
    insert_record(&repo, &staged_record(second.clone(), "second.png"));
    let second_claim = claim("", second.clone(), true);
    assert!(cancel_staged_record(
        &repo,
        &second_claim.tenant,
        "thread",
        &staged_doc_id(&second_id).unwrap(),
        &second,
        &second_claim,
    )
    .unwrap());
    let entries = vec![
        StoredAttachment {
            part: first.clone(),
            path: "first.png".into(),
        },
        StoredAttachment {
            part: second.clone(),
            path: "second.png".into(),
        },
    ];
    assert!(claim_staged_refs(
        &crate::core::runtime::DispatchCtx::ambient(),
        "thread",
        &config,
        "user:req",
        &entries,
    )
    .await
    .is_err());
    let first_claim = claim("user:req", first.clone(), false);
    assert!(!claim_staged_record(
        &repo,
        &first_claim.tenant,
        "thread",
        &staged_doc_id(&first_id).unwrap(),
        &first,
        &first_claim,
    )
    .unwrap());
    let other = claim("user:other", first.clone(), false);
    assert!(claim_staged_record(
        &repo,
        &other.tenant,
        "thread",
        &staged_doc_id(&first_id).unwrap(),
        &first,
        &other,
    )
    .is_err());
}

#[tokio::test]
async fn discarding_claimed_attachment_preserves_row_owned_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("actions");
    let opaque_id = uuid::Uuid::new_v4().simple().to_string();
    let part = image(MediaReference {
        opaque_id: Some(opaque_id.clone()),
        content_type: Some("image/png".into()),
        ..Default::default()
    });
    let repo = test_repo(&config.workspace_dir);
    insert_record(&repo, &staged_record(part.clone(), "committed.png"));
    let bound = claim("user:req", part.clone(), false);
    assert!(claim_staged_record(
        &repo,
        &bound.tenant,
        "thread",
        &staged_doc_id(&opaque_id).unwrap(),
        &part,
        &bound,
    )
    .unwrap());
    let workspace = crate::attachments::thread_action_dir(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &config.workspace_dir,
        Some("thread"),
    )
    .await
    .unwrap();
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    let file_path = workspace.join("committed.png");
    tokio::fs::write(&file_path, b"row-owned bytes")
        .await
        .unwrap();
    discard_staged_entries(
        &crate::core::runtime::DispatchCtx::ambient(),
        "thread",
        &config,
        vec![StoredAttachment {
            part: part.clone(),
            path: "committed.png".into(),
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        tokio::fs::read(file_path).await.unwrap(),
        b"row-owned bytes"
    );
    let stored = get_staged_record(&repo, &staged_doc_id(&opaque_id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored.claim, Some(bound));
}

#[path = "attachments_staged_tests.rs"]
mod staged_tests;

#[path = "attachments_native_tests.rs"]
mod native_tests;
