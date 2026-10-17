use super::*;

#[tokio::test]
async fn prepare_messages_handles_mixed_image_and_file_markers() {
    let temp = tempfile::tempdir().unwrap();
    let png_path = temp.path().join("frame.png");
    std::fs::write(
        &png_path,
        [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'],
    )
    .unwrap();

    let txt_path = temp.path().join("note.txt");
    std::fs::write(&txt_path, b"caption").unwrap();

    let messages = vec![TranscriptMessage::user(format!(
        "compare [IMAGE:{}] with [FILE:{}]",
        png_path.display(),
        txt_path.display()
    ))];

    let prepared = prepare_messages_for_provider(
        &DispatchCtx::ambient(),
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();

    assert!(prepared.contains_images);
    assert!(prepared.contains_files);
    let body = &prepared.messages[0].content;
    assert!(body.contains("[IMAGE:data:image/png;base64,"));
    assert!(body.contains("[FILE-EXTRACTED:"));
    assert!(body.contains("caption"));
}

#[test]
fn count_markers_only_inspects_latest_user_message() {
    // Regression: earlier versions summed markers across every user
    // role in history, so an N-turn thread that attached 1 file per
    // turn eventually exceeded max_files even though no single turn
    // attached more than 1. Per-turn semantics: count only the latest
    // user message.
    let history = vec![
        TranscriptMessage::user(
            "[FILE:/tmp/a.txt] [FILE:/tmp/b.txt] [FILE:/tmp/c.txt] [FILE:/tmp/d.txt]".to_string(),
        ),
        TranscriptMessage::assistant("ok"),
        TranscriptMessage::user("now just one [FILE:/tmp/e.txt]".to_string()),
    ];
    assert_eq!(count_file_markers(&history), 1);

    let history_no_new_files = vec![
        TranscriptMessage::user("[FILE:/tmp/a.txt] [FILE:/tmp/b.txt]".to_string()),
        TranscriptMessage::assistant("ok"),
        TranscriptMessage::user("no attachments this turn".to_string()),
    ];
    assert_eq!(count_file_markers(&history_no_new_files), 0);

    // Same semantics for the image counter.
    let image_history = vec![
        TranscriptMessage::user("[IMAGE:/tmp/1.png] [IMAGE:/tmp/2.png]".to_string()),
        TranscriptMessage::assistant("ok"),
        TranscriptMessage::user("plain text only".to_string()),
    ];
    assert_eq!(count_image_markers(&image_history), 0);
}

#[test]
fn for_untrusted_channel_input_disables_file_markers_and_remote_fetch() {
    // The hardened constructor used by the channel runtime and triage
    // arm: any [FILE:…] marker must be rejected before disk reads, and
    // remote fetch must be off so an attacker can't pivot to URLs.
    let cfg = MultimodalFileConfig::for_untrusted_channel_input();
    assert_eq!(
        cfg.max_files, 0,
        "max_files must be the 0 sentinel so prepare_messages_for_provider short-circuits"
    );
    assert!(
        !cfg.allow_remote_fetch,
        "remote fetch must stay disabled on untrusted channel turns"
    );
}

#[tokio::test]
async fn prepare_messages_rejects_absolute_file_marker_under_untrusted_channel_config() {
    // Regression: a Slack/Discord/Telegram user sending an
    // `[FILE:/etc/passwd]` in a normal message must NOT trigger any
    // disk read. The pre-clamp gate inside prepare_messages_for_provider
    // honours `max_files: 0` and returns TooManyFiles before
    // normalize_file_reference is called.
    let cfg = MultimodalFileConfig::for_untrusted_channel_input();
    let messages = vec![TranscriptMessage::user(
        "please summarise [FILE:/etc/passwd]".to_string(),
    )];
    let err = prepare_messages_for_provider(
        &DispatchCtx::ambient(),
        &messages,
        &MultimodalConfig::default(),
        &cfg,
    )
    .await
    .expect_err("absolute file marker on a channel turn must be rejected");
    assert!(
        err.to_string().contains("multimodal file limit exceeded"),
        "expected TooManyFiles, got {err}"
    );
}

#[tokio::test]
async fn prepare_messages_rejects_relative_file_marker_under_untrusted_channel_config() {
    // Same gate, relative path. Belt-and-suspenders: even a path that
    // looks "local" to the cwd would be a disk read against the server
    // process working directory if it slipped through.
    let cfg = MultimodalFileConfig::for_untrusted_channel_input();
    let messages = vec![TranscriptMessage::user(
        "what does [FILE:./relative.txt] say?".to_string(),
    )];
    let err = prepare_messages_for_provider(
        &DispatchCtx::ambient(),
        &messages,
        &MultimodalConfig::default(),
        &cfg,
    )
    .await
    .expect_err("relative file marker on a channel turn must be rejected");
    assert!(
        err.to_string().contains("multimodal file limit exceeded"),
        "expected TooManyFiles, got {err}"
    );
}

#[tokio::test]
async fn prepare_messages_under_untrusted_channel_config_passes_plain_text_through() {
    // Sanity: text with no [FILE:…] markers must still go through
    // unchanged. The hardening only rejects file-marker smuggling, not
    // ordinary channel chatter.
    let cfg = MultimodalFileConfig::for_untrusted_channel_input();
    let messages = vec![TranscriptMessage::user("hello, how are you?".to_string())];
    let prepared = prepare_messages_for_provider(
        &DispatchCtx::ambient(),
        &messages,
        &MultimodalConfig::default(),
        &cfg,
    )
    .await
    .expect("plain channel text must pass through the hardened config");
    assert!(!prepared.contains_files);
    assert!(!prepared.contains_images);
    assert_eq!(prepared.messages.len(), 1);
}

// ── Ingress attachment processing: file extraction + image sidecar ───────────

#[tokio::test]
async fn inline_file_attachments_replaces_marker_with_extracted_text() {
    // base64("hello world") = aGVsbG8gd29ybGQ=
    let msg = "summarize [FILE:data:text/plain;base64,aGVsbG8gd29ybGQ=]";
    let out = inline_file_attachments(
        &DispatchCtx::ambient(),
        msg,
        &MultimodalFileConfig::default(),
    )
    .await;
    assert!(
        out.contains("[FILE-EXTRACTED"),
        "extracted block present: {out}"
    );
    assert!(out.contains("hello world"), "extracted text inlined: {out}");
    assert!(
        !out.contains("[FILE:data:"),
        "raw data-uri marker must be gone: {out}"
    );
    assert!(out.contains("summarize"), "user text preserved: {out}");
}

#[tokio::test]
async fn inline_file_attachments_noop_without_marker() {
    let msg = "just a normal message";
    let out = inline_file_attachments(
        &DispatchCtx::ambient(),
        msg,
        &MultimodalFileConfig::default(),
    )
    .await;
    assert_eq!(out, msg);
}

#[tokio::test]
async fn stash_image_attachments_replaces_marker_with_placeholder() {
    let msg = format!("whats this [IMAGE:{TINY_PNG_DATA_URI}]");
    let out = stash_image_attachments(&msg, &MultimodalConfig::default()).await;
    assert!(out.contains("[Image:"), "placeholder present: {out}");
    assert!(out.contains("#att:"), "stash ref present: {out}");
    assert!(
        !out.contains("[IMAGE:data:"),
        "raw image marker must be gone: {out}"
    );
    assert!(
        !out.contains("base64"),
        "no base64 left in persisted form: {out}"
    );
    assert!(out.contains("whats this"), "user text preserved: {out}");
}

#[tokio::test]
async fn image_placeholder_rehydrates_to_inline_data_uri_for_provider() {
    // Ingress: stash the image in the blob store and leave a placeholder.
    let msg = format!("describe [IMAGE:{TINY_PNG_DATA_URI}]");
    let placeholdered = stash_image_attachments(&msg, &MultimodalConfig::default()).await;
    let messages = vec![TranscriptMessage::user(placeholdered)];
    assert!(has_image_placeholders(&messages), "placeholder detected");

    // Dispatch (vision model): rehydrate to an inline [IMAGE:<path>] marker that
    // points at the on-disk attachment. The index is rebuilt from disk on every
    // call (no in-memory state), so this resolves even after a process restart.
    let rehydrated = rehydrate_image_placeholders(&messages).await;
    assert_eq!(rehydrated.len(), 1);
    let content = rehydrated[0].content.clone();
    assert!(
        content.contains("[IMAGE:"),
        "rehydrated inline marker: {content}"
    );
    assert!(
        !content.contains("#att:"),
        "placeholder consumed: {content}"
    );

    // The marker carries the stashed bytes as an inline data URI.
    assert!(
        content.contains("[IMAGE:data:image/png;base64,"),
        "stashed attachment rehydrated inline: {content}"
    );

    // Round-trip: the provider prep step re-reads the file back into a data URI,
    // so the model still receives inline image bytes.
    let prepared = prepare_messages_for_provider(
        &DispatchCtx::ambient(),
        &rehydrated,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();
    assert!(
        prepared.messages[0]
            .content
            .contains("[IMAGE:data:image/png"),
        "provider sees re-encoded data URI: {}",
        prepared.messages[0].content
    );
}

#[tokio::test]
async fn rehydrate_missing_stash_id_keeps_placeholder_text() {
    // A placeholder whose id is absent from the stash (e.g. after a restart) is
    // left verbatim rather than dropped — the model still sees a text mention.
    let messages = vec![TranscriptMessage::user(
        "see [Image: image #att:deadbeefdeadbeef]".to_string(),
    )];
    let out = rehydrate_image_placeholders(&messages).await;
    assert!(out[0]
        .content
        .contains("[Image: image #att:deadbeefdeadbeef]"));
    assert!(!out[0].content.contains("[IMAGE:data:"));
}

#[tokio::test]
async fn inline_file_attachments_caps_at_max_files() {
    // base64("a")=YQ==, base64("b")=Yg==
    let msg = "[FILE:data:text/plain;base64,YQ==] [FILE:data:text/plain;base64,Yg==]";
    let cfg = MultimodalFileConfig {
        max_files: 1,
        ..MultimodalFileConfig::default()
    };
    let out = inline_file_attachments(&DispatchCtx::ambient(), msg, &cfg).await;
    // First file is extracted; the second is over the cap → placeholder, unread.
    assert!(
        out.contains("[FILE-EXTRACTED"),
        "first file extracted: {out}"
    );
    assert!(out.contains("over file limit"), "second file capped: {out}");
}

#[tokio::test]
async fn stash_image_attachments_caps_at_max_images() {
    let msg = format!("[IMAGE:{TINY_PNG_DATA_URI}]\n[IMAGE:{TINY_PNG_DATA_URI}]");
    let cfg = MultimodalConfig {
        max_images: 1,
        ..MultimodalConfig::default()
    };
    let out = stash_image_attachments(&msg, &cfg).await;
    assert!(out.contains("#att:"), "first image stashed: {out}");
    assert!(
        out.contains("over image limit"),
        "second image capped: {out}"
    );
}

#[test]
fn extract_image_placeholders_pulls_att_tokens_in_order() {
    // Forwards a user's stashed image(s) into a delegated vision sub-agent.
    let text = "look at these [Image: image #att:aaa111] and [Image: image #att:bbb222] please";
    let got = extract_image_placeholders_in_text(text);
    assert_eq!(
        got,
        vec![
            "[Image: image #att:aaa111]".to_string(),
            "[Image: image #att:bbb222]".to_string()
        ]
    );
    // A bare placeholder with no stash ref is ignored; plain text yields none.
    assert!(extract_image_placeholders_in_text("[Image: (could not be processed)]").is_empty());
    assert!(extract_image_placeholders_in_text("no images here").is_empty());
}

#[tokio::test]
async fn sweep_keeps_fresh_attachments() {
    // A freshly-written attachment (age < TTL) survives the startup sweep, and
    // the index resolves it — the core of restart-survival.
    let msg = format!("[IMAGE:{TINY_PNG_DATA_URI}]");
    let placeholdered = stash_image_attachments(&msg, &MultimodalConfig::default()).await;
    let id = placeholdered
        .split("#att:")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .map(|s| s.trim().to_string())
        .expect("placeholder carries an id");

    sweep_stale_attachments().await;

    let index = build_attachment_index(&[TranscriptMessage::user(placeholdered.clone())]).await;
    assert!(
        index.contains_key(&id),
        "fresh attachment {id} retained after sweep"
    );
}

#[test]
fn legacy_sidecar_files_stay_indexed_by_id() {
    // Conversations written before the blob stash keep `<id>.<ext>` files in the
    // attachments directory; their placeholders must still resolve.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("abc123.png"), b"png").unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("bad name.png"), b"png").unwrap();
    let index = legacy_attachment_index_in(dir.path());
    assert_eq!(index.len(), 1, "{index:?}");
    assert_eq!(index["abc123"], dir.path().join("abc123.png"));
}
