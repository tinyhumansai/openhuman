use super::*;

#[tokio::test]
async fn native_inbound_preparation_preserves_image_envelope_for_typed_turns() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    use tinychannels_bus::{
        channel::{
            ChannelRef, ContentPart, ConversationKind, ConversationRef, MediaKind, MediaReference,
            SenderRef,
        },
        module_inbound::{ChannelInboundSource, PreparedChannelInbound},
    };

    let envelope = tinychannels_bus::ChannelInboundEnvelope {
        channel: ChannelRef {
            id: "telegram".into(),
            account_id: Some("work".into()),
        },
        message_id: "provider-message-7".into(),
        conversation: ConversationRef {
            kind: ConversationKind::Dm,
            id: "chat-12".into(),
            thread_id: Some("thread-3".into()),
            topic_id: Some("topic-4".into()),
            ..Default::default()
        },
        sender: SenderRef {
            id: "sender-2".into(),
            ..Default::default()
        },
        text: String::new(),
        parts: vec![ContentPart::Image {
            media: MediaReference {
                opaque_id: Some("opaque-module-media".into()),
                content_type: Some("image/png".into()),
                size_bytes: Some(64),
                kind: MediaKind::Image,
                ..Default::default()
            },
            caption: None,
            alt_text: Some("diagram".into()),
        }],
        ..Default::default()
    };

    let request = channel_inbound_preparation_request(
        ChannelInboundSource::Envelope {
            envelope: envelope.clone(),
            timestamp: 1,
        },
        tinychannels_bus::module_inbound::ChannelInboundPreparationMode::Typed,
    );
    let prepared: PreparedChannelInbound = crate::modules::channel_controller::prepare_inbound(
        &crate::channels::module_fixture::client().await,
        request,
    )
    .await
    .expect("native channel preparation must retain typed image input");

    assert_eq!(prepared.envelope.parts, envelope.parts);
    assert_eq!(prepared.envelope.channel, envelope.channel);
    assert_eq!(prepared.envelope.conversation, envelope.conversation);
    assert_eq!(prepared.envelope.message_id, "provider-message-7");
    assert!(prepared.message.content.is_empty());

    let legacy = crate::modules::channel_controller::prepare_inbound(
        &crate::channels::module_fixture::client().await,
        channel_inbound_preparation_request(
            ChannelInboundSource::Envelope {
                envelope,
                timestamp: 1,
            },
            Default::default(),
        ),
    )
    .await;
    assert!(
        legacy.is_err(),
        "legacy callers must still reject rich-only input"
    );
}

#[tokio::test]
async fn native_typed_preparation_preserves_ordered_voice_and_file_parts() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    use tinychannels_bus::{
        channel::{
            ChannelRef, ContentPart, ConversationKind, ConversationRef, MediaKind, MediaReference,
            SenderRef, VoiceNote, VoiceTranscript, VoiceTranscriptSource, VoiceTranscriptState,
        },
        module_inbound::{ChannelInboundPreparationMode, ChannelInboundSource},
    };

    let envelope = tinychannels_bus::ChannelInboundEnvelope {
        channel: ChannelRef {
            id: "telegram".into(),
            account_id: Some("work".into()),
        },
        message_id: "provider-message-voice-file".into(),
        conversation: ConversationRef {
            kind: ConversationKind::Dm,
            id: "chat-12".into(),
            thread_id: Some("thread-3".into()),
            topic_id: Some("topic-4".into()),
            ..Default::default()
        },
        sender: SenderRef {
            id: "sender-2".into(),
            ..Default::default()
        },
        text: "before".into(),
        parts: vec![
            ContentPart::Text {
                text: "before".into(),
                markdown_dialect: None,
            },
            ContentPart::VoiceNote {
                voice_note: VoiceNote {
                    transcript: Some(VoiceTranscript {
                        text: "spoken words".into(),
                        language: Some("en".into()),
                        state: VoiceTranscriptState::Complete,
                        source: VoiceTranscriptSource::HostStt,
                    }),
                    ..Default::default()
                },
            },
            ContentPart::File {
                media: MediaReference {
                    opaque_id: Some("scoped-file-ref".into()),
                    file_name: Some("notes.txt".into()),
                    content_type: Some("text/plain".into()),
                    kind: MediaKind::Document,
                    ..Default::default()
                },
                file_name: Some("notes.txt".into()),
                caption: Some("meeting notes".into()),
            },
            ContentPart::Text {
                text: "after".into(),
                markdown_dialect: None,
            },
        ],
        ..Default::default()
    };

    let prepared = crate::modules::channel_controller::prepare_inbound(
        &crate::channels::module_fixture::client().await,
        channel_inbound_preparation_request(
            ChannelInboundSource::Envelope {
                envelope: envelope.clone(),
                timestamp: 2,
            },
            ChannelInboundPreparationMode::Typed,
        ),
    )
    .await
    .expect("typed preparation retains the full provider envelope");

    assert_eq!(prepared.envelope, envelope);
    assert_eq!(prepared.message.content, "before\nspoken words\nafter");
    assert_eq!(
        prepared.envelope.channel.account_id.as_deref(),
        Some("work")
    );
    assert_eq!(
        prepared.envelope.conversation.thread_id.as_deref(),
        Some("thread-3")
    );
    assert_eq!(
        prepared.envelope.conversation.topic_id.as_deref(),
        Some("topic-4")
    );
    assert!(matches!(
        prepared.envelope.parts.get(2),
        Some(ContentPart::File { media, .. }) if media.opaque_id.as_deref() == Some("scoped-file-ref")
    ));
}

#[test]
fn voice_message_console_preview_never_includes_transcript_text() {
    let envelope = tinychannels_bus::ChannelInboundEnvelope {
        parts: vec![tinychannels_bus::ContentPart::VoiceNote {
            voice_note: tinychannels_bus::VoiceNote {
                transcript: Some(tinychannels_bus::VoiceTranscript {
                    text: "private dictated words".into(),
                    language: Some("en".into()),
                    state: tinychannels_bus::VoiceTranscriptState::Complete,
                    source: tinychannels_bus::VoiceTranscriptSource::HostStt,
                }),
                ..Default::default()
            },
        }],
        ..Default::default()
    };

    assert_eq!(
        inbound_console_preview(&envelope, "private dictated words"),
        "[voice message]"
    );
}

#[test]
fn ordinary_text_console_preview_remains_readable() {
    let envelope = tinychannels_bus::ChannelInboundEnvelope::default();

    assert_eq!(inbound_console_preview(&envelope, "hello"), "hello");
}

fn transient_reply_channel(
    failures: usize,
) -> (
    std::sync::Arc<crate::channels::runtime::TestChannel>,
    crate::channels::runtime::ChannelHandle,
) {
    let mut channel = crate::channels::runtime::TestChannel::new("telegram");
    channel.send_error = Some("error sending request".to_owned());
    channel
        .send_failures_remaining
        .store(failures, std::sync::atomic::Ordering::SeqCst);
    let channel = std::sync::Arc::new(channel);
    let handle = crate::channels::runtime::ChannelHandle::Test(std::sync::Arc::clone(&channel));
    (channel, handle)
}

fn reply_message() -> SendMessage {
    SendMessage::new("hello", "chat-1").with_idempotency_key("reply:telegram:msg-1")
}

/// #8590: a transient send failure is retried and the reply lands.
#[tokio::test]
async fn failed_reply_retries_transient_errors_then_succeeds() {
    let (channel, handle) = transient_reply_channel(2);

    send_reply_with_retry(&handle, &reply_message())
        .await
        .expect("reply must land after transient failures");

    assert_eq!(
        channel.send_count.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "two failed attempts plus the delivering one"
    );
    let sent = channel.sent.lock().expect("test send lock");
    assert_eq!(sent.len(), 1, "exactly one reply is recorded");
    assert_eq!(
        sent[0].idempotency_key.as_deref(),
        Some("reply:telegram:msg-1"),
        "retries re-send the same message and key"
    );
}

/// #8590: a persistently transient failure exhausts the retries.
#[tokio::test]
async fn failed_reply_gives_up_after_retries() {
    let (channel, handle) = transient_reply_channel(usize::MAX);

    let err = send_reply_with_retry(&handle, &reply_message())
        .await
        .expect_err("reply must fail once retries are exhausted");
    assert!(
        err.to_string().contains("error sending request"),
        "last send error surfaces: {err}"
    );

    assert_eq!(
        channel.send_count.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "no more than the bounded attempts are made"
    );
    assert!(
        channel.sent.lock().expect("test send lock").is_empty(),
        "no reply is recorded"
    );
}

/// #8590: a permanent failure (provider 4xx) fails fast without retrying.
#[tokio::test]
async fn failed_reply_does_not_retry_permanent_errors() {
    let mut test_channel = crate::channels::runtime::TestChannel::new("telegram");
    test_channel.send_error =
        Some("Telegram API error (400 Bad Request): chat not found".to_owned());
    let channel = std::sync::Arc::new(test_channel);
    let handle = crate::channels::runtime::ChannelHandle::Test(std::sync::Arc::clone(&channel));

    send_reply_with_retry(&handle, &reply_message())
        .await
        .expect_err("permanent failure must surface");

    assert_eq!(
        channel.send_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "permanent errors are not retried"
    );
}
