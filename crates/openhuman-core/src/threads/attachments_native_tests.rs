//! Native channel attachments: staging, authorised reads, commit and cleanup.

use super::*;
use tinychannels_bus::MediaKind;

const THREAD: &str = "telegram:account-a:conversation-native";

struct Fixture {
    _dir: tempfile::TempDir,
    config: crate::config::Config,
    ctx: crate::core::runtime::DispatchCtx,
    lease: NativeStagedLease,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("actions");
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let lease = NativeStagedLease::new(ctx.clone(), THREAD.into(), config.clone(), Vec::new());
    Fixture {
        _dir: dir,
        config,
        ctx,
        lease,
    }
}

fn source(kind: MediaKind) -> ContentPart {
    let media = MediaReference {
        kind,
        ..Default::default()
    };
    match kind {
        MediaKind::Image => ContentPart::Image {
            media,
            caption: None,
            alt_text: None,
        },
        _ => ContentPart::File {
            media,
            file_name: Some("notes.txt".into()),
            caption: None,
        },
    }
}

async fn stage(
    f: &Fixture,
    input: &ContentPart,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> ContentPart {
    stage_native_channel_part(
        &f.ctx, THREAD, &f.config, input, name, mime, bytes, &f.lease,
    )
    .await
    .unwrap()
}

async fn read(
    f: &Fixture,
    id: &str,
    current: &[ContentPart],
    history: bool,
) -> Result<(Vec<u8>, String, String), String> {
    read_authorized_native_attachment(&f.ctx, &f.config, THREAD, id, current, history)
        .await
        .map(|r| (r.bytes, r.mime, r.path))
}

fn opaque(part: &ContentPart) -> String {
    opaque_media(part).unwrap().opaque_id.clone().unwrap()
}

#[tokio::test]
async fn staged_native_image_is_readable_only_with_a_current_grant() {
    let f = fixture();
    let part = stage(
        &f,
        &source(MediaKind::Image),
        "pic.png",
        "image/png",
        b"png-bytes",
    )
    .await;
    let id = opaque(&part);

    let (bytes, mime, path) = read(&f, &id, std::slice::from_ref(&part), false)
        .await
        .unwrap();
    assert_eq!(bytes, b"png-bytes");
    assert_eq!(mime, "image/png");
    assert!(!path.is_empty());

    let no_grant = read(&f, &id, &[], false).await.unwrap_err();
    assert!(
        no_grant.contains("lacks a current or committed host grant"),
        "{no_grant}"
    );

    let other_thread = read_authorized_native_attachment(
        &f.ctx,
        &f.config,
        "other-thread",
        &id,
        std::slice::from_ref(&part),
        false,
    )
    .await
    .map(|_| ())
    .unwrap_err();
    assert!(
        other_thread.contains("outside the active session scope"),
        "{other_thread}"
    );

    let unknown = uuid::Uuid::new_v4().simple().to_string();
    let missing = read(&f, &unknown, &[], false).await.unwrap_err();
    assert!(missing.contains("not registered"), "{missing}");
    assert!(read(&f, "not-a-uuid", &[], false).await.is_err());
}

#[tokio::test]
async fn committed_native_attachment_stays_readable_for_history_only_when_allowed() {
    let f = fixture();
    let part = stage(
        &f,
        &source(MediaKind::Image),
        "pic.png",
        "image/png",
        b"history-bytes",
    )
    .await;
    let id = opaque(&part);
    let repo = staged_repo(&f.ctx, &f.config.workspace_dir).unwrap();
    let tenant = tenant_for(&f.ctx).unwrap();
    let claim = StagedAttachmentClaim {
        tenant: tenant.clone(),
        thread_id: THREAD.into(),
        message_id: "message-1".into(),
        part: part.clone(),
        canceled: false,
    };
    claim_staged_record(
        &repo,
        &tenant,
        THREAD,
        &staged_doc_id(&id).unwrap(),
        &part,
        &claim,
    )
    .unwrap();
    f.lease.mark_committed();

    let denied = read(&f, &id, &[], false).await.unwrap_err();
    assert!(
        denied.contains("lacks a current or committed host grant"),
        "{denied}"
    );
    let (bytes, _, _) = read(&f, &id, &[], true).await.unwrap();
    assert_eq!(bytes, b"history-bytes");
}

#[tokio::test]
async fn canceled_native_attachment_cannot_be_read_even_with_a_grant() {
    let f = fixture();
    let part = stage(
        &f,
        &source(MediaKind::Image),
        "pic.png",
        "image/png",
        b"cancel-me",
    )
    .await;
    let id = opaque(&part);
    f.lease.discard().await.unwrap();
    // A second discard is a no-op.
    f.lease.discard().await.unwrap();
    let err = read(&f, &id, std::slice::from_ref(&part), true)
        .await
        .unwrap_err();
    assert!(err.contains("canceled"), "{err}");
    assert!(
        f.lease.track(part).is_err(),
        "a discarded lease accepts no new parts"
    );
}

#[tokio::test]
async fn native_attachment_policy_is_rechecked_when_it_is_read() {
    let mut f = fixture();
    let part = stage(
        &f,
        &source(MediaKind::Document),
        "notes.txt",
        "text/plain",
        b"hello file",
    )
    .await;
    let id = opaque(&part);
    let current = std::slice::from_ref(&part);

    // Allowed while the MIME type stays on the allow-list...
    let (bytes, _, _) = read(&f, &id, current, false).await.unwrap();
    assert_eq!(bytes, b"hello file");

    // ...and rejected once the operator removes it.
    f.config.multimodal_files.allowed_mime_types.clear();
    let err = read(&f, &id, current, false).await.unwrap_err();
    assert!(
        err.contains("MIME is no longer allowed") || err.contains("limit"),
        "{err}"
    );
}

#[tokio::test]
async fn staging_rejects_empty_input_and_unstageable_kinds_without_leaving_files() {
    let f = fixture();
    let empty = stage_native_channel_part(
        &f.ctx,
        THREAD,
        &f.config,
        &source(MediaKind::Image),
        "x.png",
        "image/png",
        b"",
        &f.lease,
    )
    .await
    .unwrap_err();
    assert!(empty.contains("non-empty"), "{empty}");
    let no_thread = stage_native_channel_part(
        &f.ctx,
        " ",
        &f.config,
        &source(MediaKind::Image),
        "x.png",
        "image/png",
        b"x",
        &f.lease,
    )
    .await
    .unwrap_err();
    assert!(no_thread.contains("non-empty"), "{no_thread}");

    let text = ContentPart::Text {
        text: "not media".into(),
        markdown_dialect: None,
    };
    let err = stage_native_channel_part(
        &f.ctx,
        THREAD,
        &f.config,
        &text,
        "x.png",
        "image/png",
        b"bytes",
        &f.lease,
    )
    .await
    .unwrap_err();
    assert!(err.contains("cannot be staged"), "{err}");
    let workspace = crate::attachments::thread_action_dir(
        &f.ctx,
        &f.config,
        &f.config.workspace_dir,
        Some(THREAD),
    )
    .await
    .unwrap();
    let leftovers = walk_files(&workspace);
    assert!(
        leftovers.is_empty(),
        "failed staging removed its file: {leftovers:?}"
    );

    // A lease that is no longer active refuses the staged part and cleans up.
    f.lease.discard().await.unwrap();
    let inactive = stage_native_channel_part(
        &f.ctx,
        THREAD,
        &f.config,
        &source(MediaKind::Image),
        "y.png",
        "image/png",
        b"bytes",
        &f.lease,
    )
    .await
    .unwrap_err();
    assert!(inactive.contains("no longer active"), "{inactive}");
}

fn walk_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test]
async fn bounded_staged_reads_reject_oversize_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let file = root.join("blob.bin");
    std::fs::write(&file, b"0123456789").unwrap();
    assert_eq!(
        read_staged_file_bounded(&file, &root, 10).await.unwrap(),
        b"0123456789"
    );
    let too_big = read_staged_file_bounded(&file, &root, 9).await.unwrap_err();
    assert!(too_big.contains("byte limit"), "{too_big}");
    let missing = read_staged_file_bounded(&root.join("missing.bin"), &root, 10)
        .await
        .unwrap_err();
    assert!(missing.contains("unavailable"), "{missing}");
}

#[tokio::test]
async fn discarding_staged_native_parts_removes_files_and_empty_input_is_a_no_op() {
    let f = fixture();
    commit_native_channel_parts(&f.ctx, THREAD, &[], "message-0")
        .await
        .unwrap();

    let part = stage(
        &f,
        &source(MediaKind::Image),
        "gone.png",
        "image/png",
        b"discard",
    )
    .await;
    let workspace = crate::attachments::thread_action_dir(
        &f.ctx,
        &f.config,
        &f.config.workspace_dir,
        Some(THREAD),
    )
    .await
    .unwrap();
    assert_eq!(walk_files(&workspace).len(), 1);
    drop(part);
    f.lease.discard().await.unwrap();
    assert!(walk_files(&workspace).is_empty());
}

#[test]
fn committed_message_selects_only_the_staged_parts_it_references() {
    use tinyinference_llm::message::{ContentBlock, MediaRef, Message};
    let staged = |id: &str, kind: MediaKind| {
        let mut part = source(kind);
        match &mut part {
            ContentPart::Image { media, .. } | ContentPart::File { media, .. } => {
                media.opaque_id = Some(id.into());
            }
            _ => unreachable!(),
        }
        part
    };
    let image = staged("img-1", MediaKind::Image);
    let file = staged("file-1", MediaKind::Document);
    let unused = staged("unused-1", MediaKind::Image);
    let all = [image.clone(), file.clone(), unused];

    let message = crate::agent::message_convert::user_message_from_parts(
        "ctx",
        &[
            ContentPart::Image {
                media: MediaReference {
                    opaque_id: Some("img-1".into()),
                    path: Some("openhuman-staged://img-1".into()),
                    content_type: Some("image/png".into()),
                    kind: MediaKind::Image,
                    ..Default::default()
                },
                caption: None,
                alt_text: None,
            },
            ContentPart::File {
                media: MediaReference {
                    opaque_id: Some("file-1".into()),
                    path: Some("openhuman-staged://file-1".into()),
                    content_type: Some("text/plain".into()),
                    kind: MediaKind::Document,
                    ..Default::default()
                },
                file_name: Some("notes.txt".into()),
                caption: None,
            },
        ],
    )
    .unwrap();
    assert!(matches!(&message, Message::User(user)
        if user.content.iter().any(|b| matches!(b, ContentBlock::Image(_)))
            && user.content.iter().any(|b| matches!(b, ContentBlock::Document(MediaRef::Path { .. })))));
    assert_eq!(
        native_parts_in_committed_message(&all, &message),
        vec![image, file]
    );
    assert!(native_parts_in_committed_message(&all, &Message::assistant("text")).is_empty());
}

fn stored_file(name: &str, mime: &str, size: usize) -> Attachment {
    Attachment {
        name: name.into(),
        path: format!("uploads/{name}"),
        mime: mime.into(),
        size_bytes: size,
    }
}

#[test]
fn typed_parts_pair_with_staged_files_and_drop_provider_locations() {
    let located = |kind| MediaReference {
        kind,
        opaque_id: Some("provider-id".into()),
        path: Some("/provider/path".into()),
        url: Some("https://provider.invalid/x".into()),
        local_cache_path: Some("/cache/x".into()),
        ..Default::default()
    };
    let parts = vec![
        ContentPart::Text {
            text: "hello".into(),
            markdown_dialect: None,
        },
        ContentPart::Image {
            media: located(MediaKind::Image),
            caption: Some("c".into()),
            alt_text: Some("a".into()),
        },
        ContentPart::Audio {
            media: located(MediaKind::Audio),
            codec: Some("opus".into()),
            duration_ms: Some(1200),
            title: Some("song".into()),
            transcript: None,
        },
        ContentPart::Video {
            media: located(MediaKind::Video),
            caption: None,
            duration_ms: Some(900),
            thumbnail: Some(located(MediaKind::Image)),
        },
        ContentPart::File {
            media: located(MediaKind::Document),
            file_name: Some("report.pdf".into()),
            caption: None,
        },
    ];
    let files = vec![
        stored_file("a.png", "image/png", 11),
        stored_file("b.opus", "audio/ogg", 22),
        stored_file("c.mp4", "video/mp4", 33),
        stored_file("d.pdf", "application/pdf", 44),
    ];
    let entries = from_staged_parts(&parts, &files).unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(
        entries
            .iter()
            .map(|(_, path)| path.as_str())
            .collect::<Vec<_>>(),
        [
            "uploads/a.png",
            "uploads/b.opus",
            "uploads/c.mp4",
            "uploads/d.pdf"
        ]
    );
    for ((part, _), (mime, size)) in entries.iter().zip([
        ("image/png", 11u64),
        ("audio/ogg", 22),
        ("video/mp4", 33),
        ("application/pdf", 44),
    ]) {
        let media = opaque_media(part).unwrap();
        assert_ne!(
            media.opaque_id.as_deref(),
            Some("provider-id"),
            "fresh opaque id"
        );
        assert!(media.path.is_none() && media.url.is_none() && media.local_cache_path.is_none());
        assert_eq!(media.content_type.as_deref(), Some(mime));
        assert_eq!(media.size_bytes, Some(size));
    }
    assert!(
        matches!(&entries[0].0, ContentPart::Image { caption: Some(c), alt_text: Some(a), .. } if c == "c" && a == "a")
    );
    assert!(
        matches!(&entries[1].0, ContentPart::Audio { codec: Some(c), duration_ms: Some(1200), .. } if c == "opus")
    );
    match &entries[2].0 {
        ContentPart::Video {
            thumbnail: Some(thumb),
            duration_ms: Some(900),
            ..
        } => {
            assert!(thumb.path.is_none() && thumb.url.is_none());
            assert!(thumb.local_cache_path.is_none() && thumb.opaque_id.is_none());
        }
        other => panic!("video expected: {other:?}"),
    }
    match &entries[3].0 {
        ContentPart::File {
            media,
            file_name: Some(name),
            ..
        } => {
            assert_eq!(name, "report.pdf");
            assert_eq!(media.file_name.as_deref(), Some("report.pdf"));
        }
        other => panic!("file expected: {other:?}"),
    }

    // The private path map round-trips through message metadata, and the
    // public projection never exposes it.
    let mut metadata = serde_json::json!(null);
    let public = save(&mut metadata, entries.clone());
    assert_eq!(public.len(), 4);
    assert_eq!(public_refs(&metadata), public);
    strip_private_metadata(&mut metadata);
    assert!(public_refs(&metadata).is_empty());
}

#[test]
fn typed_part_pairing_rejects_count_mismatches_and_unsupported_kinds() {
    let image = source(MediaKind::Image);
    let err = from_staged_parts(std::slice::from_ref(&image), &[]).unwrap_err();
    assert!(err.contains("media ordering"), "{err}");
    // Voice notes carry no staged file and are skipped.
    let voice = ContentPart::VoiceNote {
        voice_note: Default::default(),
    };
    assert!(from_staged_parts(&[voice], &[]).unwrap().is_empty());
}
