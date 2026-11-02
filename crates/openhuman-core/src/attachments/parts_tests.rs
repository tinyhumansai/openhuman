use super::*;
use tinychannels_bus::MediaKind;

fn media(url: &str) -> MediaReference {
    MediaReference {
        url: Some(url.to_string()),
        ..Default::default()
    }
}

fn image(url: &str) -> ContentPart {
    ContentPart::Image {
        media: media(url),
        caption: None,
        alt_text: None,
    }
}

fn file(url: &str) -> ContentPart {
    ContentPart::File {
        media: media(url),
        file_name: Some("a.pdf".into()),
        caption: None,
    }
}

#[test]
fn no_parts_returns_text_untouched() {
    assert_eq!(fold_into_message("  hi  ", &[]).unwrap(), "  hi  ");
}

#[test]
fn images_and_files_become_markers_after_the_trimmed_text() {
    let out = fold_into_message(
        " look ",
        &[
            image("data:image/png;base64,AAA"),
            file("https://x.test/a.pdf"),
        ],
    )
    .unwrap();
    assert_eq!(
        out,
        "look [IMAGE:data:image/png;base64,AAA] [FILE:https://x.test/a.pdf]"
    );
}

#[test]
fn attachments_without_text_are_just_markers() {
    let out = fold_into_message("", &[image("data:image/png;base64,AAA")]).unwrap();
    assert_eq!(out, "[IMAGE:data:image/png;base64,AAA]");
}

#[test]
fn video_and_audio_ride_the_file_marker() {
    let video = ContentPart::Video {
        media: media("data:video/mp4;base64,AAA"),
        caption: None,
        duration_ms: None,
        thumbnail: None,
    };
    let audio = ContentPart::Audio {
        media: media("data:audio/mpeg;base64,AAA"),
        codec: None,
        duration_ms: None,
        title: None,
        transcript: None,
    };
    let out = fold_into_message("", &[video, audio]).unwrap();
    assert_eq!(
        out,
        "[FILE:data:video/mp4;base64,AAA] [FILE:data:audio/mpeg;base64,AAA]"
    );
}

#[test]
fn typed_voice_notes_keep_audio_out_of_model_text() {
    let part = ContentPart::VoiceNote {
        voice_note: tinychannels_bus::VoiceNote {
            media: MediaReference {
                opaque_id: Some("note-typed".into()),
                content_type: Some("audio/wav".into()),
                kind: MediaKind::Audio,
                ..Default::default()
            },
            duration_ms: Some(5000),
            ..Default::default()
        },
    };
    assert_eq!(
        fold_into_message("heard words", &[part]).unwrap(),
        "heard words"
    );
}

#[test]
fn text_parts_join_the_message_before_markers() {
    let parts = [
        image("data:image/png;base64,AAA"),
        ContentPart::Text {
            text: " extra ".into(),
            markdown_dialect: None,
        },
    ];
    assert_eq!(
        fold_into_message("hi", &parts).unwrap(),
        "hi extra [IMAGE:data:image/png;base64,AAA]"
    );
}

#[test]
fn local_paths_are_refused_even_with_a_url() {
    let mut m = media("data:image/png;base64,AAA");
    m.path = Some("/etc/passwd".into());
    let part = ContentPart::Image {
        media: m,
        caption: None,
        alt_text: None,
    };
    assert_eq!(
        fold_into_message("", &[part]),
        Err(AttachmentPartError::LocalPath { index: 0 })
    );
    let mut m = media("data:image/png;base64,AAA");
    m.local_cache_path = Some("/tmp/x".into());
    let part = ContentPart::File {
        media: m,
        file_name: None,
        caption: None,
    };
    assert_eq!(
        fold_into_message("", &[part]),
        Err(AttachmentPartError::LocalPath { index: 0 })
    );
}

#[test]
fn media_needs_a_data_or_http_url() {
    let none = ContentPart::Image {
        media: MediaReference::default(),
        caption: None,
        alt_text: None,
    };
    assert_eq!(
        fold_into_message("", &[none]),
        Err(AttachmentPartError::NoSource { index: 0 })
    );
    assert_eq!(
        fold_into_message("", &[image("file:///etc/passwd")]),
        Err(AttachmentPartError::NoSource { index: 0 })
    );
}

#[test]
fn urls_that_could_break_the_marker_are_refused() {
    assert_eq!(
        fold_into_message("", &[image("data:image/png;base64,AA] [FILE:/etc/passwd")]),
        Err(AttachmentPartError::BadUrl { index: 0 })
    );
    assert_eq!(
        fold_into_message("", &[image("https://x.test/a b")]),
        Err(AttachmentPartError::BadUrl { index: 0 })
    );
}

#[test]
fn non_media_parts_are_unsupported() {
    let sticker = ContentPart::Sticker {
        media: MediaReference {
            kind: MediaKind::Image,
            ..media("data:image/webp;base64,AAA")
        },
        pack_id: None,
        sticker_id: None,
        emoji: None,
        alt_text: None,
    };
    assert_eq!(
        fold_into_message("", &[image("data:image/png;base64,AAA"), sticker]),
        Err(AttachmentPartError::UnsupportedKind {
            index: 1,
            kind: "sticker"
        })
    );
}

#[test]
fn too_many_parts_are_refused() {
    let parts: Vec<ContentPart> = (0..33)
        .map(|_| image("data:image/png;base64,AAA"))
        .collect();
    assert_eq!(
        fold_into_message("", &parts),
        Err(AttachmentPartError::TooMany { count: 33 })
    );
}

#[test]
fn error_messages_name_the_attachment() {
    let msg = AttachmentPartError::LocalPath { index: 2 }.to_string();
    assert!(msg.contains("attachment 2"), "{msg}");
}
