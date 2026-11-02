use super::*;
use base64::Engine as _;

#[tokio::test]
async fn legacy_sidecar_rehydrates_into_an_authorized_acting_original() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    // The acting workspace is provisioned before enabling its strict boundary.
    tokio::fs::create_dir_all(&cfg.action_dir).await.unwrap();
    cfg.autonomy.enabled = true;
    let stash = cfg.workspace_dir.join("attachments");
    tokio::fs::create_dir_all(&stash).await.unwrap();
    let source = stash.join("legacy.png");
    let bytes = b"\x89PNG\r\n\x1a\noriginal";
    tokio::fs::write(&source, bytes).await.unwrap();
    let scope = AttachmentAccessScope::default();
    assert!(resolve_path(&cfg, source.to_str().unwrap(), &scope)
        .await
        .is_err());
    let index = std::collections::HashMap::from([("legacy".into(), source.clone())]);
    let prompt = markers::rehydrate_placeholders_in_text("[Image: old #att:legacy]", &index);
    let paths = markers::parse_image_markers(&prompt).1;
    assert_eq!(paths, vec![source.to_string_lossy().into_owned()]);
    let migrated = legacy::migrate_from(&cfg, &paths[0], &stash, &scope)
        .await
        .unwrap()
        .unwrap();
    assert!(migrated.starts_with("uploads/legacy-sidecars/"));
    assert_eq!(
        legacy::migrate_from(&cfg, &paths[0], &stash, &scope)
            .await
            .unwrap()
            .unwrap(),
        migrated
    );
    let acting = resolve_path(&cfg, &migrated, &scope).await.unwrap();
    assert_eq!(tokio::fs::read(acting).await.unwrap(), bytes);
    assert_eq!(tokio::fs::read(&source).await.unwrap(), bytes);
    let message =
        crate::agent::message_convert::user_message_from_text(&format!("[IMAGE:{migrated}]"));
    assert_eq!(
        image_references(&message, &[]),
        vec![format!("[IMAGE:{migrated}]")]
    );
}

#[tokio::test]
async fn legacy_migration_does_not_grant_private_file_or_disabled_image_access() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    let scope = AttachmentAccessScope::default();
    let stash = cfg.workspace_dir.join("attachments");
    tokio::fs::create_dir_all(&stash).await.unwrap();
    let source = stash.join("legacy.png");
    tokio::fs::write(&source, b"\x89PNG\r\n\x1a\noriginal")
        .await
        .unwrap();
    let external_scope = AttachmentAccessScope {
        external_channel: true,
        workspace: None,
        ..Default::default()
    };
    assert!(
        legacy::migrate_from(&cfg, source.to_str().unwrap(), &stash, &external_scope)
            .await
            .is_err()
    );
    let unrelated = cfg.workspace_dir.join("private.png");
    assert!(
        legacy::migrate_from(&cfg, unrelated.to_str().unwrap(), &stash, &scope)
            .await
            .unwrap()
            .is_none()
    );
    cfg.multimodal_files.max_files = 0;
    assert!(
        legacy::migrate_from(&cfg, source.to_str().unwrap(), &stash, &scope)
            .await
            .is_err()
    );
    cfg.multimodal_files.max_files = 4;
    cfg.multimodal.max_images = 0;
    assert!(
        legacy::migrate_from(&cfg, source.to_str().unwrap(), &stash, &scope)
            .await
            .is_err()
    );
    cfg.multimodal.max_images = 4;
    tokio::fs::write(&source, b"private text masquerading as a sidecar")
        .await
        .unwrap();
    assert!(
        legacy::migrate_from(&cfg, source.to_str().unwrap(), &stash, &scope)
            .await
            .is_err()
    );
    let forbidden = temp.path().join(".ssh").join("attachments");
    tokio::fs::create_dir_all(&forbidden).await.unwrap();
    let credential = forbidden.join("legacy.png");
    tokio::fs::write(&credential, b"\x89PNG\r\n\x1a\nprivate")
        .await
        .unwrap();
    assert!(
        legacy::migrate_from(&cfg, credential.to_str().unwrap(), &forbidden, &scope)
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn legacy_migration_rejects_symlink_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let scope = AttachmentAccessScope::default();
    let stash = cfg.workspace_dir.join("attachments");
    tokio::fs::create_dir_all(&stash).await.unwrap();
    let outside = temp.path().join("outside.png");
    tokio::fs::write(&outside, b"\x89PNG\r\n\x1a\nprivate")
        .await
        .unwrap();
    let link = stash.join("legacy.png");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(
        legacy::migrate_from(&cfg, link.to_str().unwrap(), &stash, &scope)
            .await
            .is_err()
    );
}

#[test]
fn enrichment_preserves_typed_media_and_image_forwarding() {
    use tinyinference_llm::message::{ContentBlock, ImageRef, MediaRef, Message, UserMessage};
    let media = MediaRef::Path {
        path: "uploads/thread/id/media".into(),
        media_type: Some("audio/wav".into()),
    };
    let blocks = vec![
        ContentBlock::Text("caption".into()),
        ContentBlock::Image(ImageRef {
            url: "uploads/thread/id/image.png".into(),
            mime_type: None,
        }),
        ContentBlock::Audio(media.clone()),
        ContentBlock::Video(media.clone()),
        ContentBlock::Document(media),
    ];
    let input = Message::User(UserMessage {
        content: blocks.clone(),
    });
    let images = image_references(&input, &[]);
    assert_eq!(images, vec!["[IMAGE:uploads/thread/id/image.png]"]);
    let Message::User(enriched) = enrich_input(input, "caption", "context\ncaption") else {
        panic!("user")
    };
    assert_eq!(&enriched.content[1..], &blocks[1..]);
    assert_eq!(
        enriched.content[0],
        ContentBlock::Text("context\ncaption".into())
    );
    assert!(!should_forward_parent_images(&images[0]));
    assert!(should_forward_parent_images(
        "analyze the user's attachments"
    ));
}

#[tokio::test]
async fn forbidden_acting_root_is_rejected_before_directory_creation() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = config(temp.path());
    config.action_dir = temp.path().join(".ssh").join("new-workspace");
    assert!(save(
        &config,
        "thread",
        "file",
        "text/plain",
        b"data",
        &AttachmentAccessScope::default()
    )
    .await
    .is_err());
    assert!(!temp.path().join(".ssh").exists());
}

fn config(root: &Path) -> Config {
    Config {
        action_dir: root.join("acting"),
        workspace_dir: root.join("internal"),
        ..Config::default()
    }
}

#[tokio::test]
async fn ready_media_artifact_in_files_folder_can_be_attached() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = config(temp.path());
    let files = temp.path().join("Files");
    tokio::fs::create_dir_all(&files).await.unwrap();
    config.files_dir_override = Some(files.clone());
    config.autonomy.enabled = true;

    let (artifact, path) = crate::artifacts::create_artifact(
        &config.workspace_dir,
        &files,
        crate::artifacts::ArtifactKind::Image,
        "Generated image",
        "png",
        None,
    )
    .await
    .unwrap();
    tokio::fs::write(&path, b"image bytes").await.unwrap();
    crate::artifacts::finalize_artifact(&config.workspace_dir, &artifact.id, 11, None)
        .await
        .unwrap();

    let resolved = resolve_path(
        &config,
        &path.to_string_lossy(),
        &AttachmentAccessScope::default(),
    )
    .await
    .unwrap();
    assert_eq!(resolved, path.canonicalize().unwrap());

    let unrelated = files.join("unrelated.png");
    tokio::fs::write(&unrelated, b"not a filed media artifact")
        .await
        .unwrap();
    assert!(resolve_path(
        &config,
        &unrelated.to_string_lossy(),
        &AttachmentAccessScope::default(),
    )
    .await
    .is_err());
}

#[tokio::test]
async fn arbitrary_original_is_durable_and_reference_contains_no_payload() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let bytes = b"\0binary ZIP audio video original";
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let input = format!(
        "Inspect [FILE:data:application/octet-stream;name=Report%20%CE%B1.zip;base64,{encoded}]"
    );
    let staged = stage(
        &input,
        "../../unsafe/thread",
        &config,
        &AttachmentAccessScope::default(),
    )
    .await
    .unwrap();
    assert!(!staged.contains(&encoded));
    let (text, files) = parse(&staged);
    assert_eq!(text.trim(), "Inspect");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "Report α.zip");
    assert!(files[0].path.starts_with("uploads/"));
    assert!(safe_relative_path(&files[0].path));
    assert_eq!(
        tokio::fs::read(config.action_dir.join(&files[0].path))
            .await
            .unwrap(),
        bytes
    );
    assert_eq!(
        stage(
            &staged,
            "thread",
            &config,
            &AttachmentAccessScope::default()
        )
        .await
        .unwrap(),
        staged
    );
    assert_eq!(parse(&files[0].marker()).1, files);
}

#[tokio::test]
async fn repeated_names_do_not_overwrite_and_escape_names_are_sanitized() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let a = save(
        &config,
        "thread",
        "../../same.zip",
        "application/zip",
        b"first",
        &AttachmentAccessScope::default(),
    )
    .await
    .unwrap();
    let b = save(
        &config,
        "thread",
        "../../same.zip",
        "application/zip",
        b"second",
        &AttachmentAccessScope::default(),
    )
    .await
    .unwrap();
    assert_ne!(a.path, b.path);
    assert_eq!(a.name, "../../same.zip");
    assert_eq!(Path::new(&a.path).file_name().unwrap(), "same.zip");
    let unicode_name = filename(&"界".repeat(100));
    assert!(unicode_name.len() <= 180);
    assert!(unicode_name.len() >= 177);
    assert!(safe_relative_path(&a.path));
    assert_eq!(
        tokio::fs::read(config.action_dir.join(a.path))
            .await
            .unwrap(),
        b"first"
    );
}

#[tokio::test]
async fn staging_preserves_interleaved_captions_and_media_order() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let source = "first [FILE:data:text/plain;name=one.txt;base64,YQ==] second [IMAGE:data:image/png;name=two.png;base64,iVBORw0KGgo=] third";
    let staged = stage(source, "thread", &config, &AttachmentAccessScope::default())
        .await
        .unwrap();
    let parts = segments(&staged);
    assert!(matches!(&parts[0], Segment::Text(text) if text == "first "));
    assert!(matches!(&parts[1], Segment::Attachment(file) if file.name == "one.txt"));
    assert!(matches!(&parts[2], Segment::Text(text) if text == " second "));
    assert!(matches!(&parts[3], Segment::Attachment(file) if file.name == "two.png"));
    assert!(matches!(&parts[4], Segment::Text(text) if text == " third"));
}

#[tokio::test]
async fn hard_zero_and_count_gate_precede_reads() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = config(temp.path());
    config.multimodal_files.max_files = 0;
    assert!(stage(
        "[FILE:/missing]",
        "thread",
        &config,
        &AttachmentAccessScope::default()
    )
    .await
    .is_err());
    assert!(stage(
        "[IMAGE:/missing]",
        "thread",
        &config,
        &AttachmentAccessScope::default()
    )
    .await
    .is_err());
    assert!(!config.action_dir.exists());
    assert!(!safe_relative_path("../escape"));
    assert!(!safe_relative_path("/absolute"));
    assert!(!safe_relative_path("uploads\\escape"));
}

#[tokio::test]
async fn vision_without_explicit_references_fails_before_configuration_or_inference() {
    assert!(!has_resolvable_image(
        &crate::core::runtime::DispatchCtx::ambient(),
        "Describe a filename mentioned only in prose: picture.png",
        None,
        None
    )
    .await
    .unwrap());
    assert!(delegation_prompt(
        &crate::core::runtime::DispatchCtx::ambient(),
        "task",
        &serde_json::json!({ "image_paths": [42] }),
        None,
        None
    )
    .await
    .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn upload_symlink_is_rejected_without_writing_outside() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    tokio::fs::create_dir_all(&config.action_dir).await.unwrap();
    let outside = temp.path().join("outside");
    tokio::fs::create_dir(&outside).await.unwrap();
    std::os::unix::fs::symlink(&outside, config.action_dir.join("uploads")).unwrap();
    assert!(save(
        &config,
        "thread",
        "file",
        "application/octet-stream",
        b"original",
        &AttachmentAccessScope::default()
    )
    .await
    .is_err());
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
}

#[tokio::test]
async fn explicit_workspace_wins_over_config_action_dir() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let scoped = temp.path().join("scoped");
    let scope = AttachmentAccessScope {
        external_channel: false,
        workspace: Some(scoped.clone()),
        ..Default::default()
    };
    let attachment = save(&config, "thread", "file", "text/plain", b"original", &scope)
        .await
        .unwrap();
    assert!(scoped.join(attachment.path).exists());
    assert!(!config.action_dir.exists());
}

#[test]
fn enriched_plain_turn_reloads_with_identical_text_blocks() {
    use tinyinference_llm::message::Message;
    let enriched = enrich_input(Message::user("caption"), "caption", "context\ncaption");
    let row = tinyagents_runtime::transcript::message_to_native_transcript(&enriched).unwrap();
    let replayed = crate::agent::message_convert::chat_message_to_message(&row);
    assert_eq!(replayed, enriched);
}
