//! End-to-end typed media preparation against the linked TinyChannels module:
//! bytes are uploaded to the module's scoped store, read back through the
//! chunked media port, staged on the host and converted to a model message.

use super::*;
use crate::channels::runtime::test_support::{linked_telegram_runtime, LinkedTelegramRuntime};
use std::collections::HashMap;
use std::sync::Arc;
use tinychannels_bus::{ChannelRef, ConversationKind, ConversationRef};

const THREAD: &str = "telegram:telegram:conversation-42";

/// A valid 1x1 PNG: the module checks image bytes against their declared type.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0xd, 0xa, 0x1a, 0xa, 0x0, 0x0, 0x0, 0xd, 0x49, 0x48, 0x44, 0x52, 0x0,
    0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x1, 0x8, 0x6, 0x0, 0x0, 0x0, 0x1f, 0x15, 0xc4, 0x89, 0x0, 0x0,
    0x0, 0xb, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x0, 0x2, 0x0, 0x0, 0x5, 0x0, 0x1,
    0x7a, 0x5e, 0xab, 0x3f, 0x0, 0x0, 0x0, 0x0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

pub(in crate::channels::runtime::dispatch::processor) fn envelope(
    parts: Vec<ContentPart>,
) -> ChannelInboundEnvelope {
    ChannelInboundEnvelope {
        channel: ChannelRef {
            id: "telegram".into(),
            account_id: Some("telegram".into()),
        },
        conversation: ConversationRef {
            kind: ConversationKind::Dm,
            id: "42".into(),
            ..Default::default()
        },
        message_id: "provider-message-1".into(),
        parts,
        ..Default::default()
    }
}

pub(in crate::channels::runtime::dispatch::processor) fn runtime_context(
    objects: Arc<HashMap<String, ModuleObjectClient>>,
) -> ChannelRuntimeContext {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "ok",
        ]));
    ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(HashMap::new()),
        module_objects: objects,
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(model)),
        default_provider: Arc::new("default".into()),
        tools_registry: Arc::new(Vec::new()),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("prompt"),
        model: Arc::new("model".into()),
        temperature: 0.0,
        max_tool_iterations: 1,
        turn_model_source_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
        route_overrides: Arc::new(std::sync::Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(std::path::PathBuf::from("/tmp")),
        message_timeout_secs: 30,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    }
}

fn host_config(dir: &tempfile::TempDir) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("actions");
    config
}

async fn upload(
    linked: &LinkedTelegramRuntime,
    envelope: &ChannelInboundEnvelope,
    kind: MediaKind,
    content_type: &str,
    name: &str,
    bytes: &[u8],
) -> MediaReference {
    let object = linked
        .module_objects()
        .get("telegram")
        .cloned()
        .expect("telegram object");
    let scope = media_scope(envelope).unwrap();
    let handle: tinychannels_bus::MediaUploadHandle = object
        .call_confidential(
            tinychannels_bus::methods::BEGIN_MEDIA_UPLOAD,
            (tinychannels_bus::MediaUploadBegin {
                instance: "telegram".into(),
                scope,
                kind,
                content_type: content_type.into(),
                file_name: Some(name.into()),
                duration_ms: None,
                codec: None,
                byte_count: bytes.len(),
            },),
        )
        .await
        .expect("scoped upload begins");
    object
        .call_confidential::<u32>(
            tinychannels_bus::methods::APPEND_MEDIA_UPLOAD_CHUNK,
            (tinychannels_bus::MediaUploadChunk {
                handle: handle.clone(),
                sequence: 0,
                bytes: bytes.to_vec(),
            },),
        )
        .await
        .expect("chunk accepted");
    object
        .call_confidential(
            tinychannels_bus::methods::FINISH_MEDIA_UPLOAD,
            (tinychannels_bus::MediaUploadFinish(handle),),
        )
        .await
        .expect("upload finalizes into an opaque reference")
}

fn image_part(media: MediaReference) -> ContentPart {
    ContentPart::Image {
        media,
        caption: None,
        alt_text: None,
    }
}

fn file_part(media: MediaReference) -> ContentPart {
    ContentPart::File {
        media,
        file_name: Some("notes.txt".into()),
        caption: None,
    }
}

#[tokio::test]
async fn typed_media_is_read_from_the_module_staged_and_converted_to_a_model_message() {
    let linked = linked_telegram_runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let config = host_config(&dir);
    let ctx = runtime_context(linked.module_objects());
    let host = DispatchCtx::ambient();

    let probe = envelope(Vec::new());
    let image = upload(
        &linked,
        &probe,
        MediaKind::Image,
        "image/png",
        "pic.png",
        PNG,
    )
    .await;
    let file = upload(
        &linked,
        &probe,
        MediaKind::Document,
        "text/plain",
        "notes.txt",
        b"file-payload",
    )
    .await;
    let voice = ContentPart::VoiceNote {
        voice_note: tinychannels_bus::VoiceNote {
            transcript: Some(tinychannels_bus::VoiceTranscript {
                text: "  spoken words  ".into(),
                language: None,
                state: tinychannels_bus::VoiceTranscriptState::Complete,
                source: tinychannels_bus::VoiceTranscriptSource::HostStt,
            }),
            ..Default::default()
        },
    };
    let inbound = envelope(vec![
        ContentPart::Text {
            text: "look".into(),
            markdown_dialect: None,
        },
        voice,
        image_part(image),
        file_part(file),
    ]);

    let resolved =
        resolve_typed_message(&ctx, &host, &config, &inbound, THREAD, "[Channel context]")
            .await
            .expect("typed media resolves")
            .expect("media envelopes need preparation");

    assert_eq!(resolved.staged_parts.len(), 2);
    for part in &resolved.staged_parts {
        let media = crate::threads::attachments::has_opaque_ref(part);
        assert!(media, "staged parts carry a host opaque reference");
    }
    let Message::User(user) = &resolved.message else {
        panic!("user message expected")
    };
    use tinyinference_llm::message::{ContentBlock, MediaRef};
    assert!(matches!(&user.content[0], ContentBlock::Text(t) if t == "[Channel context]"));
    assert!(user
        .content
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(t) if t == "spoken words")));
    assert!(user
        .content
        .iter()
        .any(|b| matches!(b, ContentBlock::Image(i) if i.url.starts_with("openhuman-staged://"))));
    assert!(user.content.iter().any(|b| matches!(
        b,
        ContentBlock::Document(MediaRef::Path { path, .. }) if path.starts_with("openhuman-staged://")
    )));

    // The staged bytes are exactly what the module held.
    let staged_image = &resolved.staged_parts[0];
    let id = match staged_image {
        ContentPart::Image { media, .. } => media.opaque_id.clone().unwrap(),
        other => panic!("unexpected {other:?}"),
    };
    let read = crate::threads::attachments::read_authorized_native_attachment(
        &host,
        &config,
        THREAD,
        &id,
        std::slice::from_ref(staged_image),
        false,
    )
    .await
    .map_err(|e| e.to_string())
    .map(|r| r.bytes)
    .unwrap();
    assert_eq!(read, PNG);
    resolved.staged_lease.discard().await.unwrap();
}

#[tokio::test]
async fn text_only_envelopes_need_no_media_preparation() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(Arc::new(HashMap::new()));
    let inbound = envelope(vec![ContentPart::Text {
        text: "hi".into(),
        markdown_dialect: None,
    }]);
    let out = resolve_typed_message(
        &ctx,
        &DispatchCtx::ambient(),
        &host_config(&dir),
        &inbound,
        THREAD,
        "c",
    )
    .await
    .unwrap();
    assert!(out.is_none());
}

fn png_ref() -> MediaReference {
    MediaReference {
        opaque_id: Some("opaque-1".into()),
        content_type: Some("image/png".into()),
        size_bytes: Some(8),
        kind: MediaKind::Image,
        ..Default::default()
    }
}

async fn reject(
    ctx: &ChannelRuntimeContext,
    config: &crate::config::Config,
    inbound: &ChannelInboundEnvelope,
) -> String {
    resolve_typed_message(ctx, &DispatchCtx::ambient(), config, inbound, THREAD, "c")
        .await
        .map(|_| ())
        .unwrap_err()
}

#[tokio::test]
async fn typed_media_is_rejected_before_any_module_read_when_policy_fails() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(Arc::new(HashMap::new()));
    let config = host_config(&dir);

    let audio = envelope(vec![ContentPart::Audio {
        media: png_ref(),
        codec: None,
        duration_ms: None,
        title: None,
        transcript: None,
    }]);
    assert!(reject(&ctx, &config, &audio)
        .await
        .contains("not supported"));

    let mut no_account = envelope(vec![image_part(png_ref())]);
    no_account.channel.account_id = None;
    assert!(reject(&ctx, &config, &no_account)
        .await
        .contains("account scope"));

    let mut bad_ref = png_ref();
    bad_ref.path = Some("/etc/passwd".into());
    assert!(reject(&ctx, &config, &envelope(vec![image_part(bad_ref)]))
        .await
        .contains("opaque"));

    let mut file_ref = png_ref();
    file_ref.kind = MediaKind::Document;
    file_ref.content_type = Some("text/plain".into());
    let mut bad_file = file_ref.clone();
    bad_file.url = Some("https://attacker.invalid/x".into());
    assert!(reject(&ctx, &config, &envelope(vec![file_part(bad_file)]))
        .await
        .contains("opaque"));

    let mut no_images = config.clone();
    no_images.multimodal.max_images = 0;
    assert!(
        reject(&ctx, &no_images, &envelope(vec![image_part(png_ref())]))
            .await
            .contains("image attachments")
    );

    let mut no_files = config.clone();
    no_files.multimodal_files.max_files = 0;
    assert!(
        reject(&ctx, &no_files, &envelope(vec![file_part(file_ref)]))
            .await
            .contains("file attachments")
    );

    // Valid parts but the configured module instance is not running.
    assert!(
        reject(&ctx, &config, &envelope(vec![image_part(png_ref())]))
            .await
            .contains("module is unavailable")
    );
}

#[tokio::test]
async fn module_read_failures_and_metadata_mismatches_reject_the_turn() {
    let linked = linked_telegram_runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let config = host_config(&dir);
    let ctx = runtime_context(linked.module_objects());
    let probe = envelope(Vec::new());

    // The module has no such reference in this turn's scope.
    let unknown = envelope(vec![image_part(png_ref())]);
    let err = reject(&ctx, &config, &unknown).await;
    assert!(
        err.contains("could not be resolved") || err.contains("mismatch"),
        "{err}"
    );

    // The envelope's declared content type disagrees with what the module stored.
    let stored = upload(
        &linked,
        &probe,
        MediaKind::Image,
        "image/png",
        "pic.png",
        PNG,
    )
    .await;
    let mut lying = stored.clone();
    lying.content_type = Some("image/jpeg".into());
    let err = reject(&ctx, &config, &envelope(vec![image_part(lying)])).await;
    assert!(
        err.contains("could not be resolved") || err.contains("does not match"),
        "{err}"
    );

    // The declared size disagrees with the stored byte count.
    let mut wrong_size = stored;
    wrong_size.size_bytes = Some(3);
    let err = reject(&ctx, &config, &envelope(vec![image_part(wrong_size)])).await;
    assert!(
        err.contains("could not be resolved") || err.contains("does not match"),
        "{err}"
    );
}

#[test]
fn rejection_logging_returns_the_error_text_unchanged() {
    assert_eq!(log_rejection("stage", "plain error".into()), "plain error");
    for error in [
        ModuleCallError::Unavailable,
        ModuleCallError::IncompatibleContract,
        ModuleCallError::TransportFailed,
        ModuleCallError::ModuleFault,
    ] {
        assert_eq!(
            log_module_call_rejection("method", error, "fallback text"),
            "fallback text"
        );
    }
}

#[test]
fn reference_validation_covers_kind_size_and_mime_rules() {
    let mut media = png_ref();
    media.size_bytes = Some(0);
    assert!(validate_reference(&media, true, 1024, &[]).is_err());
    media = png_ref();
    media.content_type = None;
    assert!(validate_reference(&media, true, 1024, &[]).is_err());
    media = png_ref();
    media.kind = MediaKind::Document;
    assert!(validate_reference(&media, true, 1024, &[]).is_err());
    media = png_ref();
    media.opaque_id =
        Some("x".repeat(tinychannels_bus::module_media::MEDIA_UPLOAD_BEGIN_MAX_BYTES + 1));
    assert!(validate_reference(&media, true, 1024, &[]).is_err());

    let mut file = png_ref();
    file.kind = MediaKind::Image;
    file.content_type = Some("text/plain".into());
    assert!(validate_reference(&file, false, 1024, &["text/plain".into()]).is_err());
    file.kind = MediaKind::Unknown;
    assert!(validate_reference(&file, false, 1024, &["text/plain".into()]).is_ok());

    let mut incomplete = envelope(Vec::new());
    incomplete.message_id.clear();
    assert!(media_scope(&incomplete).is_err());
    assert!(!requires_typed_preparation(&envelope(vec![
        ContentPart::Text {
            text: "t".into(),
            markdown_dialect: None
        }
    ])));
}
