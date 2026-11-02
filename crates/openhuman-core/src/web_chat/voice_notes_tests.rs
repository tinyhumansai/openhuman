use super::*;

fn ambient() -> crate::core::runtime::DispatchCtx {
    crate::core::runtime::DispatchCtx::ambient()
}

#[test]
fn stored_note_reads_back_with_mime() {
    let tmp = tempfile::tempdir().unwrap();
    let mime = store_note(
        &ambient(),
        tmp.path(),
        "note-1",
        "audio/ogg;codecs=opus",
        b"abc",
    )
    .unwrap();
    assert_eq!(mime, "audio/ogg");
    let (bytes, mime) = read_note(&ambient(), tmp.path(), "note-1")
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"abc");
    assert_eq!(mime, "audio/ogg");
}

#[test]
fn restoring_under_another_mime_drops_the_old_file() {
    let tmp = tempfile::tempdir().unwrap();
    store_note(&ambient(), tmp.path(), "n", "audio/webm", b"one").unwrap();
    store_note(&ambient(), tmp.path(), "n", "audio/mpeg", b"two").unwrap();
    let (bytes, mime) = read_note(&ambient(), tmp.path(), "n").unwrap().unwrap();
    assert_eq!((bytes.as_slice(), mime), (b"two".as_slice(), "audio/mpeg"));
    let blobs = blob_store(&ambient(), tmp.path()).unwrap();
    let old = storage::block_on(async move { blobs.head("voice_notes/n.webm").await }).unwrap();
    assert!(
        old.is_none(),
        "the earlier extension's blob must be replaced"
    );
}

#[test]
fn missing_note_is_none() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(read_note(&ambient(), tmp.path(), "nope").unwrap().is_none());
}

#[test]
fn path_traversal_ids_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(store_note(&ambient(), tmp.path(), "../evil", "audio/webm", b"x").is_err());
    assert!(read_note(&ambient(), tmp.path(), "a/b").is_err());
    assert!(!is_valid_note_id(""));
    assert!(!is_valid_note_id(&"a".repeat(129)));
}

#[test]
fn empty_and_oversized_audio_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(store_note(&ambient(), tmp.path(), "e", "audio/webm", b"").is_err());
    let big = vec![0u8; MAX_NOTE_BYTES + 1];
    assert!(store_note(&ambient(), tmp.path(), "big", "audio/webm", &big).is_err());
}

#[test]
fn reply_metadata_exists_only_after_the_reply_audio_is_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    assert!(reply_note_metadata(&ctx, tmp.path(), "req/1").is_none());
    store_note(
        &ambient(),
        tmp.path(),
        &reply_note_id("req/1"),
        "audio/mpeg",
        b"mp3",
    )
    .unwrap();
    let meta = reply_note_metadata(&ctx, tmp.path(), "req/1").unwrap();
    assert_eq!(meta["noteId"], "reply-req_1");
    assert_eq!(meta["mime"], "audio/mpeg");
}

#[test]
fn synthesized_reply_metadata_preserves_its_viseme_timeline() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    assert!(store_synthesized_reply(
        &ambient(),
        tmp.path(),
        "request-1",
        "YWJj",
        "audio/mpeg",
    ));
    persist_reply_visemes(
        &ctx,
        tmp.path(),
        "request-1",
        &[
            tinyinference_voice::VisemeFrame {
                viseme: "aa".into(),
                start_ms: 0,
                end_ms: 120,
            },
            tinyinference_voice::VisemeFrame {
                viseme: "PP".into(),
                start_ms: 120,
                end_ms: 220,
            },
        ],
    )
    .unwrap();

    let meta = reply_note_metadata(&ctx, tmp.path(), "request-1").unwrap();
    assert_eq!(
        meta["visemes"],
        serde_json::json!([
            {"viseme": "aa", "start_ms": 0, "end_ms": 120},
            {"viseme": "PP", "start_ms": 120, "end_ms": 220}
        ]),
        "replayed assistant audio needs the original TTS timeline for mascot lip-sync"
    );
}

#[test]
fn legacy_viseme_timeline_remains_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let id = reply_note_id("legacy");
    store_note(&ambient(), tmp.path(), &id, "audio/mpeg", b"audio").unwrap();
    std::fs::create_dir_all(tmp.path().join("voice_notes")).unwrap();
    std::fs::write(
        tmp.path()
            .join("voice_notes")
            .join(format!("{id}.visemes.json")),
        serde_json::to_vec(&serde_json::json!([{"viseme": "aa", "start_ms": 0, "end_ms": 100}]))
            .unwrap(),
    )
    .unwrap();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let meta = reply_note_metadata(&ctx, tmp.path(), "legacy").unwrap();
    assert_eq!(meta["visemes"][0]["viseme"], "aa");
}

#[test]
fn rpc_round_trip_and_bad_base64() {
    let tmp = tempfile::tempdir().unwrap();
    let out = store_from_rpc(&ambient(), tmp.path(), "rt", "audio/webm", "YWJj").unwrap();
    assert_eq!(out.value["mime"], "audio/webm");
    let read = read_for_rpc(&ambient(), tmp.path(), "rt").unwrap().value;
    assert_eq!(read["audio_base64"], "YWJj");
    assert!(read_for_rpc(&ambient(), tmp.path(), "none").unwrap().value["audio_base64"].is_null());
    assert!(store_from_rpc(&ambient(), tmp.path(), "bad", "audio/webm", "!!!").is_err());
}

#[test]
fn typed_voice_media_must_match_its_durable_note_reference() {
    let tmp = tempfile::tempdir().unwrap();
    store_note(
        &ambient(),
        tmp.path(),
        "note-typed",
        "audio/wav",
        &[0, 1, 2],
    )
    .unwrap();
    let media = |note_id: &str, content_type: &str| tinychannels_bus::MediaReference {
        opaque_id: Some(note_id.into()),
        content_type: Some(content_type.into()),
        kind: tinychannels_bus::MediaKind::Audio,
        ..Default::default()
    };

    assert!(validate_typed_media(
        &ambient(),
        tmp.path(),
        "note-typed",
        &media("note-typed", "audio/wav")
    )
    .is_ok());
    assert!(validate_typed_media(
        &ambient(),
        tmp.path(),
        "note-typed",
        &media("other-note", "audio/wav")
    )
    .is_err());
    assert!(validate_typed_media(
        &ambient(),
        tmp.path(),
        "note-typed",
        &media("note-typed", "audio/mpeg")
    )
    .is_err());
    assert!(validate_typed_media(
        &ambient(),
        tmp.path(),
        "missing",
        &media("missing", "audio/wav")
    )
    .is_err());
}

#[test]
fn synthesized_reply_is_stored_only_with_decodable_audio() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(!store_synthesized_reply(
        &ambient(),
        tmp.path(),
        "r1",
        "",
        "audio/mpeg"
    ));
    assert!(!store_synthesized_reply(
        &ambient(),
        tmp.path(),
        "r1",
        "***",
        "audio/mpeg"
    ));
    assert!(store_synthesized_reply(
        &ambient(),
        tmp.path(),
        "r1",
        "YWJj",
        "audio/mpeg"
    ));
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    assert!(reply_note_metadata(&ctx, tmp.path(), "r1").is_some());
}

#[test]
fn reply_mode_decides_when_the_reply_is_spoken() {
    use crate::config::schema::VoiceReplyMode;
    let mut config = crate::config::Config::default();
    // Default is mirror: only a voice-note turn is answered aloud.
    assert!(!should_speak_reply(&config, "t", None, Some("keyboard")));
    assert!(should_speak_reply(
        &config,
        "t",
        None,
        Some(VOICE_NOTE_SOURCE)
    ));
    assert!(should_speak_reply(&config, "t", Some(true), None));
    config.channel_voice.reply_mode = VoiceReplyMode::Always;
    assert!(should_speak_reply(&config, "t", None, None));
    config
        .channel_voice
        .threads
        .insert("t".into(), VoiceReplyMode::Never);
    assert!(!should_speak_reply(
        &config,
        "t",
        None,
        Some(VOICE_NOTE_SOURCE)
    ));
}

#[test]
fn a_note_written_as_a_file_before_the_blob_store_still_reads() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("voice_notes")).unwrap();
    std::fs::write(tmp.path().join("voice_notes/old.mp3"), b"legacy").unwrap();
    let (bytes, mime) = read_note(&ambient(), tmp.path(), "old").unwrap().unwrap();
    assert_eq!(
        (bytes.as_slice(), mime),
        (b"legacy".as_slice(), "audio/mpeg")
    );
    assert!(reply_note_metadata(&ambient(), tmp.path(), "none").is_none());
}
