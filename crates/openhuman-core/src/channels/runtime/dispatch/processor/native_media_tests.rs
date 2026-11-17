use super::*;
use tinychannels_bus::{ChannelInboundEnvelope, MediaKind};

fn reference() -> MediaReference {
    MediaReference {
        opaque_id: Some("opaque-media".into()),
        content_type: Some("image/png".into()),
        size_bytes: Some(32),
        kind: MediaKind::Image,
        ..Default::default()
    }
}

#[test]
fn native_media_requires_an_opaque_reference_and_rejects_provider_paths() {
    let mut media = reference();
    media.opaque_id = None;
    assert!(validate_reference(&media, true, 1024, &[]).is_err());

    let mut media = reference();
    media.path = Some("/etc/passwd".into());
    assert!(validate_reference(&media, true, 1024, &[]).is_err());

    let mut media = reference();
    media.url = Some("https://attacker.invalid/media".into());
    assert!(validate_reference(&media, true, 1024, &[]).is_err());

    let mut media = reference();
    media.local_cache_path = Some("/tmp/private".into());
    assert!(validate_reference(&media, true, 1024, &[]).is_err());
}

#[test]
fn native_media_rejects_declared_oversize_and_unsupported_mime() {
    let mut media = reference();
    media.size_bytes = Some(1025);
    assert!(validate_reference(&media, true, 1024, &[]).is_err());

    let mut media = reference();
    media.content_type = Some("image/svg+xml".into());
    assert!(validate_reference(&media, true, 1024, &[]).is_err());

    let mut file = reference();
    file.kind = MediaKind::Document;
    file.content_type = Some("application/x-executable".into());
    assert!(validate_reference(&file, false, 1024, &["text/plain".into()]).is_err());
}

#[test]
fn typed_native_media_is_selected_only_from_envelope_parts() {
    let envelope = ChannelInboundEnvelope {
        parts: vec![ContentPart::Image {
            media: reference(),
            caption: None,
            alt_text: None,
        }],
        ..Default::default()
    };
    assert!(requires_typed_preparation(&envelope));

    let text_only = ChannelInboundEnvelope {
        parts: vec![ContentPart::Text {
            text: "ordinary text".into(),
            markdown_dialect: None,
        }],
        ..Default::default()
    };
    assert!(!requires_typed_preparation(&text_only));
}

#[test]
fn native_audio_video_and_incomplete_voice_are_rejected_before_media_reads() {
    let audio = ContentPart::Audio {
        media: reference(),
        codec: None,
        duration_ms: None,
        title: None,
        transcript: None,
    };
    assert!(validate_native_parts(&[audio]).is_err());

    let video = ContentPart::Video {
        media: reference(),
        caption: None,
        duration_ms: None,
        thumbnail: None,
    };
    assert!(validate_native_parts(&[video]).is_err());

    let pending_voice = ContentPart::VoiceNote {
        voice_note: tinychannels_bus::VoiceNote {
            transcript: Some(tinychannels_bus::VoiceTranscript {
                text: "partial".into(),
                language: None,
                state: tinychannels_bus::VoiceTranscriptState::Pending,
                source: tinychannels_bus::VoiceTranscriptSource::HostStt,
            }),
            ..Default::default()
        },
    };
    assert!(validate_native_parts(&[pending_voice]).is_err());

    for transcript in [
        None,
        Some(tinychannels_bus::VoiceTranscript {
            text: "unavailable transcript".into(),
            language: None,
            state: tinychannels_bus::VoiceTranscriptState::Unavailable,
            source: tinychannels_bus::VoiceTranscriptSource::HostStt,
        }),
        Some(tinychannels_bus::VoiceTranscript {
            text: " \n ".into(),
            language: None,
            state: tinychannels_bus::VoiceTranscriptState::Complete,
            source: tinychannels_bus::VoiceTranscriptSource::HostStt,
        }),
    ] {
        let unavailable = ContentPart::VoiceNote {
            voice_note: tinychannels_bus::VoiceNote {
                transcript,
                ..Default::default()
            },
        };
        assert!(validate_native_parts(&[unavailable]).is_err());
    }
}

#[test]
fn module_media_scope_preserves_account_full_conversation_and_provider_turn() {
    let envelope = ChannelInboundEnvelope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some("telegram:work".into()),
        },
        conversation: tinychannels_bus::ConversationRef {
            kind: tinychannels_bus::ConversationKind::Thread,
            id: "chat-42".into(),
            thread_id: Some("thread-7".into()),
            topic_id: Some("topic-9".into()),
            ..Default::default()
        },
        message_id: "provider-message-19".into(),
        ..Default::default()
    };
    let scope = media_scope(&envelope).unwrap();
    assert_eq!(scope.configured_instance, "telegram:work");
    assert_eq!(scope.channel, envelope.channel);
    assert_eq!(scope.conversation, envelope.conversation);
    assert_eq!(scope.turn_id, "provider-message-19");

    let missing_account = ChannelInboundEnvelope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: None,
        },
        ..envelope.clone()
    };
    assert!(media_scope(&missing_account).is_err());

    let mismatched_account = ChannelInboundEnvelope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some("discord:work".into()),
        },
        ..envelope
    };
    assert!(media_scope(&mismatched_account).is_err());
}

#[test]
fn scoped_media_actual_bytes_must_match_declared_and_configured_bounds() {
    assert!(validate_actual_media_size(10, 10, 10).is_ok());
    assert!(validate_actual_media_size(11, 10, 20).is_err());
    assert!(validate_actual_media_size(10, 10, 9).is_err());
    assert!(validate_actual_media_size(0, 0, 10).is_err());
}

#[test]
fn native_typed_message_keeps_context_prefix_and_interleaved_part_order() {
    let parts = vec![
        ContentPart::Text {
            text: "first text".into(),
            markdown_dialect: None,
        },
        ContentPart::VoiceNote {
            voice_note: tinychannels_bus::VoiceNote {
                transcript: Some(tinychannels_bus::VoiceTranscript {
                    text: "spoken words".into(),
                    language: Some("en".into()),
                    state: tinychannels_bus::VoiceTranscriptState::Complete,
                    source: tinychannels_bus::VoiceTranscriptSource::HostStt,
                }),
                ..Default::default()
            },
        },
        ContentPart::Image {
            media: MediaReference {
                path: Some("openhuman-staged://image-ref".into()),
                content_type: Some("image/png".into()),
                ..reference()
            },
            caption: None,
            alt_text: None,
        },
        ContentPart::Text {
            text: "second text".into(),
            markdown_dialect: None,
        },
        ContentPart::File {
            media: MediaReference {
                kind: MediaKind::Document,
                path: Some("openhuman-staged://file-ref".into()),
                content_type: Some("text/plain".into()),
                ..reference()
            },
            file_name: Some("notes.txt".into()),
            caption: None,
        },
    ];

    let Message::User(user) = crate::agent::message_convert::user_message_from_parts(
        "[Channel context] Telegram private chat",
        &parts,
    )
    .unwrap() else {
        panic!("user message expected")
    };
    assert_eq!(user.content.len(), 6);
    assert!(matches!(
        &user.content[0],
        tinyinference_llm::message::ContentBlock::Text(text)
            if text == "[Channel context] Telegram private chat"
    ));
    assert!(matches!(
        &user.content[1],
        tinyinference_llm::message::ContentBlock::Text(text) if text == "first text"
    ));
    assert!(matches!(
        &user.content[2],
        tinyinference_llm::message::ContentBlock::Text(text) if text == "spoken words"
    ));
    assert!(matches!(
        &user.content[3],
        tinyinference_llm::message::ContentBlock::Image(image)
            if image.url == "openhuman-staged://image-ref"
    ));
    assert!(matches!(
        &user.content[4],
        tinyinference_llm::message::ContentBlock::Text(text) if text == "second text"
    ));
    assert!(matches!(
        &user.content[5],
        tinyinference_llm::message::ContentBlock::Document(
            tinyinference_llm::message::MediaRef::Path { path, .. }
        ) if path == "openhuman-staged://file-ref"
    ));
}
