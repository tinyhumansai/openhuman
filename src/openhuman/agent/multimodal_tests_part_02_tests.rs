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

    let messages = vec![ChatMessage::user(format!(
        "compare [IMAGE:{}] with [FILE:{}]",
        png_path.display(),
        txt_path.display()
    ))];

    let prepared = prepare_messages_for_provider(
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
fn multimodal_file_config_effective_limits_clamp_to_safe_bounds() {
    let cfg = MultimodalFileConfig {
        max_files: 999,
        max_file_size_mb: 999,
        max_extracted_text_chars: 999_999,
        allow_remote_fetch: false,
        allowed_mime_types: vec![],
    };
    let (files, size_mb, chars) = cfg.effective_limits();
    assert_eq!(files, 16);
    assert_eq!(size_mb, 50);
    assert_eq!(chars, 200_000);

    let small = MultimodalFileConfig {
        max_files: 0,
        max_file_size_mb: 0,
        max_extracted_text_chars: 0,
        allow_remote_fetch: false,
        allowed_mime_types: vec![],
    };
    let (files, size_mb, chars) = small.effective_limits();
    assert_eq!(files, 1);
    assert_eq!(size_mb, 1);
    assert_eq!(chars, 1_000);
}

#[test]
fn multimodal_file_config_mime_allowlist_is_case_insensitive() {
    let cfg = MultimodalFileConfig::default();
    assert!(cfg.is_mime_allowed("application/pdf"));
    assert!(cfg.is_mime_allowed("APPLICATION/PDF"));
    assert!(!cfg.is_mime_allowed("application/x-executable"));
}

#[test]
fn count_markers_only_inspects_latest_user_message() {
    // Regression: earlier versions summed markers across every user
    // role in history, so an N-turn thread that attached 1 file per
    // turn eventually exceeded max_files even though no single turn
    // attached more than 1. Per-turn semantics: count only the latest
    // user message.
    let history = vec![
        ChatMessage::user(
            "[FILE:/tmp/a.txt] [FILE:/tmp/b.txt] [FILE:/tmp/c.txt] [FILE:/tmp/d.txt]".to_string(),
        ),
        ChatMessage::assistant("ok"),
        ChatMessage::user("now just one [FILE:/tmp/e.txt]".to_string()),
    ];
    assert_eq!(count_file_markers(&history), 1);
    assert!(contains_file_markers(&history));

    let history_no_new_files = vec![
        ChatMessage::user("[FILE:/tmp/a.txt] [FILE:/tmp/b.txt]".to_string()),
        ChatMessage::assistant("ok"),
        ChatMessage::user("no attachments this turn".to_string()),
    ];
    assert_eq!(count_file_markers(&history_no_new_files), 0);
    assert!(!contains_file_markers(&history_no_new_files));

    // Same semantics for the image counter.
    let image_history = vec![
        ChatMessage::user("[IMAGE:/tmp/1.png] [IMAGE:/tmp/2.png]".to_string()),
        ChatMessage::assistant("ok"),
        ChatMessage::user("plain text only".to_string()),
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
    let messages = vec![ChatMessage::user(
        "please summarise [FILE:/etc/passwd]".to_string(),
    )];
    let err = prepare_messages_for_provider(&messages, &MultimodalConfig::default(), &cfg)
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
    let messages = vec![ChatMessage::user(
        "what does [FILE:./relative.txt] say?".to_string(),
    )];
    let err = prepare_messages_for_provider(&messages, &MultimodalConfig::default(), &cfg)
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
    let messages = vec![ChatMessage::user("hello, how are you?".to_string())];
    let prepared = prepare_messages_for_provider(&messages, &MultimodalConfig::default(), &cfg)
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
    let out = inline_file_attachments(msg, &MultimodalFileConfig::default()).await;
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
    let out = inline_file_attachments(msg, &MultimodalFileConfig::default()).await;
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
async fn image_placeholder_rehydrates_to_disk_path_for_provider() {
    // Ingress: stash the image to disk and leave a placeholder.
    let msg = format!("describe [IMAGE:{TINY_PNG_DATA_URI}]");
    let placeholdered = stash_image_attachments(&msg, &MultimodalConfig::default()).await;
    let messages = vec![ChatMessage::user(placeholdered)];
    assert!(has_image_placeholders(&messages), "placeholder detected");

    // Dispatch (vision model): rehydrate to an inline [IMAGE:<path>] marker that
    // points at the on-disk attachment. The index is rebuilt from disk on every
    // call (no in-memory state), so this resolves even after a process restart.
    let rehydrated = rehydrate_image_placeholders(&messages);
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

    // The marker points at a real file on disk.
    let start = content.find("[IMAGE:").unwrap() + "[IMAGE:".len();
    let end = content[start..].find(']').unwrap() + start;
    let path = &content[start..end];
    assert!(path.ends_with(".png"), "disk path carries ext: {path}");
    assert!(
        std::path::Path::new(path).is_file(),
        "attachment persisted to disk: {path}"
    );

    // Round-trip: the provider prep step re-reads the file back into a data URI,
    // so the model still receives inline image bytes.
    let prepared = prepare_messages_for_provider(
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

#[test]
fn rehydrate_missing_stash_id_keeps_placeholder_text() {
    // A placeholder whose id is absent from the stash (e.g. after a restart) is
    // left verbatim rather than dropped — the model still sees a text mention.
    let messages = vec![ChatMessage::user(
        "see [Image: image #att:deadbeefdeadbeef]".to_string(),
    )];
    let out = rehydrate_image_placeholders(&messages);
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
    let out = inline_file_attachments(msg, &cfg).await;
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
    // the disk index resolves it — the core of restart-survival.
    let msg = format!("[IMAGE:{TINY_PNG_DATA_URI}]");
    let placeholdered = stash_image_attachments(&msg, &MultimodalConfig::default()).await;
    let id = placeholdered
        .split("#att:")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .map(|s| s.trim().to_string())
        .expect("placeholder carries an id");

    sweep_stale_attachments().await;

    let index = build_attachment_index();
    assert!(
        index.contains_key(&id),
        "fresh attachment {id} retained after sweep"
    );
}
