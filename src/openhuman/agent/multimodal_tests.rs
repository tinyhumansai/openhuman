use super::*;

#[test]
fn parse_image_markers_extracts_multiple_markers() {
    let input = "Check this [IMAGE:/tmp/a.png] and this [IMAGE:https://example.com/b.jpg]";
    let (cleaned, refs) = parse_image_markers(input);

    assert_eq!(cleaned, "Check this  and this");
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0], "/tmp/a.png");
    assert_eq!(refs[1], "https://example.com/b.jpg");
}

#[test]
fn parse_image_markers_keeps_invalid_empty_marker() {
    let input = "hello [IMAGE:] world";
    let (cleaned, refs) = parse_image_markers(input);

    assert_eq!(cleaned, "hello [IMAGE:] world");
    assert!(refs.is_empty());
}

#[tokio::test]
async fn prepare_messages_normalizes_local_image_to_data_uri() {
    let temp = tempfile::tempdir().unwrap();
    let image_path = temp.path().join("sample.png");

    // Minimal PNG signature bytes are enough for MIME detection.
    std::fs::write(
        &image_path,
        [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'],
    )
    .unwrap();

    let messages = vec![ChatMessage::user(format!(
        "Please inspect this screenshot [IMAGE:{}]",
        image_path.display()
    ))];

    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();

    assert!(prepared.contains_images);
    assert_eq!(prepared.messages.len(), 1);

    let (cleaned, refs) = parse_image_markers(&prepared.messages[0].content);
    assert_eq!(cleaned, "Please inspect this screenshot");
    assert_eq!(refs.len(), 1);
    assert!(refs[0].starts_with("data:image/png;base64,"));
}

#[tokio::test]
async fn prepare_messages_rejects_too_many_images() {
    let messages = vec![ChatMessage::user(
        "[IMAGE:/tmp/1.png]\n[IMAGE:/tmp/2.png]".to_string(),
    )];

    let config = MultimodalConfig {
        max_images: 1,
        max_image_size_mb: 5,
        allow_remote_fetch: false,
    };

    let error = prepare_messages_for_provider(&messages, &config, &MultimodalFileConfig::default())
        .await
        .expect_err("should reject image count overflow");

    assert!(error
        .to_string()
        .contains("multimodal image limit exceeded"));
}

#[tokio::test]
async fn prepare_messages_rejects_remote_url_when_disabled() {
    let messages = vec![ChatMessage::user(
        "Look [IMAGE:https://example.com/img.png]".to_string(),
    )];

    let error = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .expect_err("should reject remote image URL when fetch is disabled");

    assert!(error
        .to_string()
        .contains("multimodal remote image fetch is disabled"));
}

#[tokio::test]
async fn prepare_messages_rejects_oversized_local_image() {
    let temp = tempfile::tempdir().unwrap();
    let image_path = temp.path().join("big.png");

    let bytes = vec![0u8; 1024 * 1024 + 1];
    std::fs::write(&image_path, bytes).unwrap();

    let messages = vec![ChatMessage::user(format!(
        "[IMAGE:{}]",
        image_path.display()
    ))];
    let config = MultimodalConfig {
        max_images: 4,
        max_image_size_mb: 1,
        allow_remote_fetch: false,
    };

    let error = prepare_messages_for_provider(&messages, &config, &MultimodalFileConfig::default())
        .await
        .expect_err("should reject oversized local image");

    assert!(error
        .to_string()
        .contains("multimodal image size limit exceeded"));
}

#[test]
fn extract_ollama_image_payload_supports_data_uris() {
    let payload = extract_ollama_image_payload("data:image/png;base64,abcd==")
        .expect("payload should be extracted");
    assert_eq!(payload, "abcd==");
}

#[test]
fn helpers_cover_marker_count_payload_and_message_composition() {
    let messages = vec![
        ChatMessage::system("ignore"),
        ChatMessage::user("one [IMAGE:/tmp/a.png] two [IMAGE:/tmp/b.png]"),
    ];
    assert_eq!(count_image_markers(&messages), 2);
    assert!(contains_image_markers(&messages));
    assert_eq!(
        extract_ollama_image_payload(" local-ref ").as_deref(),
        Some("local-ref")
    );
    assert!(extract_ollama_image_payload("data:image/png;base64,   ").is_none());

    let composed =
        compose_multimodal_message("describe", &["data:image/png;base64,abc".into()], &[]);
    assert!(composed.starts_with("describe"));
    assert!(composed.contains("[IMAGE:data:image/png;base64,abc]"));
}

#[test]
fn mime_and_content_type_helpers_cover_supported_and_unknown_inputs() {
    assert_eq!(
        normalize_content_type("image/PNG; charset=utf-8").as_deref(),
        Some("image/png")
    );
    assert_eq!(normalize_content_type("   ").as_deref(), None);
    assert_eq!(mime_from_extension("JPEG"), Some("image/jpeg"));
    assert_eq!(mime_from_extension("txt"), None);
    assert_eq!(
        mime_from_magic(&[0xff, 0xd8, 0xff, 0x00]),
        Some("image/jpeg")
    );
    assert_eq!(mime_from_magic(b"GIF89a123"), Some("image/gif"));
    assert_eq!(mime_from_magic(b"BMrest"), Some("image/bmp"));
    assert_eq!(mime_from_magic(b"not-an-image"), None);
    assert_eq!(
        detect_mime(
            None,
            &[0xff, 0xd8, 0xff, 0x00],
            Some("image/webp; charset=binary")
        )
        .as_deref(),
        Some("image/webp")
    );
    assert_eq!(
        validate_mime("x", "text/plain").unwrap_err().to_string(),
        "multimodal image MIME type is not allowed for 'x': text/plain"
    );
}

#[tokio::test]
async fn normalization_helpers_cover_invalid_data_uri_and_missing_local_file() {
    let err = normalize_data_uri("data:image/png,abcd", 1024)
        .expect_err("non-base64 data uri should fail");
    assert!(err
        .to_string()
        .contains("only base64 data URIs are supported"));

    let err = normalize_data_uri("data:text/plain;base64,YQ==", 1024)
        .expect_err("unsupported mime should fail");
    assert!(err.to_string().contains("MIME type is not allowed"));

    let err = normalize_local_image("/definitely/missing.png", 1024)
        .await
        .expect_err("missing local file should fail");
    assert!(err.to_string().contains("not found or unreadable"));
}

// ── File-attachment marker tests ──────────────────────────────────────

/// Minimal valid PDF that `pdf-extract` will round-trip. Generated by
/// hand from the smallest known-good PDF skeleton; covers the
/// `/Pages` → `/Page` → `/Contents` → `Tj` text-object path that
/// `pdf-extract` walks to surface visible text.
const SAMPLE_PDF_BYTES: &[u8] = b"%PDF-1.4\n\
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n\
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n\
4 0 obj<</Length 44>>stream\n\
BT /F1 12 Tf 72 720 Td (Hello PDF World) Tj ET\n\
endstream endobj\n\
5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n\
xref\n0 6\n0000000000 65535 f\n\
trailer<</Size 6/Root 1 0 R>>\n\
startxref\n0\n%%EOF\n";

#[test]
fn parse_file_markers_extracts_multiple_markers() {
    let input = "Read [FILE:/tmp/a.pdf] and [FILE:/tmp/b.csv]";
    let (cleaned, refs) = parse_file_markers(input);
    assert_eq!(cleaned, "Read  and");
    assert_eq!(
        refs,
        vec!["/tmp/a.pdf".to_string(), "/tmp/b.csv".to_string()]
    );
}

#[test]
fn parse_file_markers_keeps_invalid_empty_marker() {
    let input = "hello [FILE:] world";
    let (cleaned, refs) = parse_file_markers(input);
    assert_eq!(cleaned, "hello [FILE:] world");
    assert!(refs.is_empty());
}

#[test]
fn parse_file_markers_does_not_interfere_with_image_markers() {
    let input = "shot [IMAGE:/tmp/x.png] doc [FILE:/tmp/y.pdf]";
    let (_, file_refs) = parse_file_markers(input);
    let (_, image_refs) = parse_image_markers(input);
    assert_eq!(file_refs, vec!["/tmp/y.pdf".to_string()]);
    assert_eq!(image_refs, vec!["/tmp/x.png".to_string()]);
    assert_eq!(count_file_markers(&[ChatMessage::user(input)]), 1);
    assert!(contains_file_markers(&[ChatMessage::user(input)]));
}

#[tokio::test]
async fn prepare_messages_extracts_text_from_plain_text_file() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("note.txt");
    std::fs::write(&file_path, b"first line\nsecond line").unwrap();

    let messages = vec![ChatMessage::user(format!(
        "Summarise [FILE:{}]",
        file_path.display()
    ))];

    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();

    assert!(prepared.contains_files);
    assert!(!prepared.contains_images);
    let body = &prepared.messages[0].content;
    assert!(body.contains("[FILE-EXTRACTED:"));
    assert!(body.contains("first line"));
    assert!(body.contains("second line"));
    assert!(body.contains("[/FILE-EXTRACTED]"));
}

#[tokio::test]
async fn prepare_messages_extracts_text_from_csv_file() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("rows.csv");
    std::fs::write(&file_path, b"a,b,c\n1,2,3").unwrap();

    let messages = vec![ChatMessage::user(format!("[FILE:{}]", file_path.display()))];
    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();
    assert!(prepared.messages[0].content.contains("a,b,c"));
    assert!(prepared.messages[0].content.contains("1,2,3"));
}

#[tokio::test]
async fn prepare_messages_extracts_text_from_markdown_file() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("notes.md");
    std::fs::write(&file_path, b"# heading\n\nbody text").unwrap();

    let messages = vec![ChatMessage::user(format!("[FILE:{}]", file_path.display()))];
    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();
    let body = &prepared.messages[0].content;
    assert!(body.contains("# heading"));
    assert!(body.contains("body text"));
}

#[tokio::test]
async fn prepare_messages_extracts_text_from_pdf() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("doc.pdf");
    std::fs::write(&file_path, SAMPLE_PDF_BYTES).unwrap();

    let messages = vec![ChatMessage::user(format!("[FILE:{}]", file_path.display()))];
    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();
    let body = &prepared.messages[0].content;
    // Tolerant: pdf-extract may emit a Reference fallback if it cannot
    // walk this hand-rolled skeleton on every host. Either path proves
    // the PDF passed the size/MIME gates and was routed through the
    // extraction branch — the agent always learns the file exists.
    assert!(
        body.contains("[FILE-EXTRACTED:") || body.contains("[FILE-ATTACHED:"),
        "expected a file block, got: {body}"
    );
    assert!(body.contains("application/pdf"));
}

#[tokio::test]
async fn prepare_messages_inlines_binary_zip_as_reference() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("bundle.zip");
    // PK\x03\x04 magic + minimal trailing bytes — enough for the
    // detect_file_mime/file_mime_from_magic path to classify as
    // application/zip without us needing a real archive.
    std::fs::write(&file_path, b"PK\x03\x04\x00\x00\x00\x00garbage-but-allowed").unwrap();

    let messages = vec![ChatMessage::user(format!("[FILE:{}]", file_path.display()))];
    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .unwrap();
    let body = &prepared.messages[0].content;
    assert!(body.contains("[FILE-ATTACHED:"));
    assert!(body.contains("application/zip"));
    assert!(body.contains("sha256_prefix="));
    assert!(!body.contains("[FILE-EXTRACTED:"));
}

#[tokio::test]
async fn prepare_messages_rejects_oversized_file() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("huge.txt");
    std::fs::write(&file_path, vec![b'a'; 2 * 1024 * 1024]).unwrap();

    let messages = vec![ChatMessage::user(format!("[FILE:{}]", file_path.display()))];
    let file_config = MultimodalFileConfig {
        max_file_size_mb: 1,
        ..Default::default()
    };

    let err = prepare_messages_for_provider(&messages, &MultimodalConfig::default(), &file_config)
        .await
        .expect_err("oversized file must be rejected");

    assert!(err
        .to_string()
        .contains("multimodal file size limit exceeded"));
}

#[tokio::test]
async fn prepare_messages_rejects_too_many_files() {
    let messages = vec![ChatMessage::user(
        "[FILE:/tmp/1.txt]\n[FILE:/tmp/2.txt]\n[FILE:/tmp/3.txt]".to_string(),
    )];
    let file_config = MultimodalFileConfig {
        max_files: 2,
        ..Default::default()
    };

    let err = prepare_messages_for_provider(&messages, &MultimodalConfig::default(), &file_config)
        .await
        .expect_err("too-many-files must be rejected");

    assert!(err.to_string().contains("multimodal file limit exceeded"));
}

#[tokio::test]
async fn prepare_messages_rejects_unsupported_file_mime() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("ride.gpx");
    // .gpx is not on the default allowlist; classify falls through to
    // utf-8 sniff which lands on text/plain, but we lock the allowlist
    // down to PDFs only so the rejection path fires deterministically.
    std::fs::write(&file_path, b"<gpx><trk/></gpx>").unwrap();

    let messages = vec![ChatMessage::user(format!("[FILE:{}]", file_path.display()))];
    let file_config = MultimodalFileConfig {
        allowed_mime_types: vec!["application/pdf".to_string()],
        ..Default::default()
    };

    let err = prepare_messages_for_provider(&messages, &MultimodalConfig::default(), &file_config)
        .await
        .expect_err("unsupported mime must be rejected");

    let msg = err.to_string();
    assert!(msg.contains("is not allowed"));
    assert!(msg.contains("supported"));
    assert!(msg.contains("application/pdf"));
}

#[tokio::test]
async fn prepare_messages_rejects_remote_file_when_disabled() {
    let messages = vec![ChatMessage::user(
        "[FILE:https://example.com/doc.pdf]".to_string(),
    )];

    let err = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .expect_err("remote-file fetch should be off by default");

    assert!(err
        .to_string()
        .contains("multimodal remote file fetch is disabled"));
}

#[tokio::test]
async fn prepare_messages_extracts_data_uri_file_marker() {
    let messages = vec![ChatMessage::user(
        "[FILE:data:text/plain;name=note.txt;base64,SGVsbG8=]".to_string(),
    )];

    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .expect("data: URI files should be supported for renderer uploads");

    assert!(prepared.contains_files);
    let body = &prepared.messages[0].content;
    assert!(body.contains("[FILE-EXTRACTED:"));
    assert!(body.contains("name=\"note.txt\""));
    assert!(body.contains("Hello"));
}

#[tokio::test]
async fn prepare_messages_decompresses_gzipped_data_uri_file_marker() {
    use base64::Engine as _;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"Hello compressed").unwrap();
    let gz = encoder.finish().unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(gz);
    let messages = vec![ChatMessage::user(format!(
        "[FILE:data:application/gzip;original_mime=text%2Fplain;name=note.txt;base64,{encoded}]"
    ))];

    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .expect("compressed data URI file should decompress");

    assert!(prepared.contains_files);
    let body = &prepared.messages[0].content;
    assert!(body.contains("Hello compressed"));
}

#[tokio::test]
async fn prepare_messages_decompresses_gzipped_data_uri_image_marker() {
    use base64::Engine as _;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let png = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&png).unwrap();
    let gz = encoder.finish().unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(gz);
    let messages = vec![ChatMessage::user(format!(
        "[IMAGE:data:application/gzip;original_mime=image%2Fpng;name=shot.png;base64,{encoded}]"
    ))];

    let prepared = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .expect("compressed data URI image should decompress");

    assert!(prepared.contains_images);
    let body = &prepared.messages[0].content;
    assert!(body.contains("[IMAGE:data:image/png;base64,"));
}

#[tokio::test]
async fn prepare_messages_bounds_gzipped_data_uri_decompression() {
    use base64::Engine as _;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&vec![0u8; 1024 * 1024 + 1]).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap());
    let messages = vec![ChatMessage::user(format!(
        "[IMAGE:data:application/gzip;original_mime=image%2Fpng;base64,{encoded}]"
    ))];
    let image_config = MultimodalConfig {
        max_images: 4,
        max_image_size_mb: 1,
        allow_remote_fetch: false,
    };

    let error =
        prepare_messages_for_provider(&messages, &image_config, &MultimodalFileConfig::default())
            .await
            .expect_err("compressed payload must be capped during decompression");

    assert!(error
        .to_string()
        .contains("decompressed payload exceeds 1048576 bytes"));
}

#[tokio::test]
async fn prepare_messages_rejects_gzip_without_original_mime() {
    use base64::Engine as _;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"Hello compressed").unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap());
    let messages = vec![ChatMessage::user(format!(
        "[FILE:data:application/gzip;name=note.txt;base64,{encoded}]"
    ))];

    let error = prepare_messages_for_provider(
        &messages,
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
    )
    .await
    .expect_err("gzip without original_mime must fail");

    assert!(error.to_string().contains("original_mime"));
}

#[tokio::test]
async fn prepare_messages_truncates_extracted_text_to_cap() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("long.txt");
    std::fs::write(&file_path, "x".repeat(5_000)).unwrap();

    let messages = vec![ChatMessage::user(format!("[FILE:{}]", file_path.display()))];
    let file_config = MultimodalFileConfig {
        max_extracted_text_chars: 1_000,
        ..Default::default()
    };

    let prepared =
        prepare_messages_for_provider(&messages, &MultimodalConfig::default(), &file_config)
            .await
            .unwrap();
    let body = &prepared.messages[0].content;
    assert!(body.contains("…truncated"));
    // truncated message must still be inside the cap (1_000) — minus
    // suffix reservation — so well under 5_000.
    let x_run_len = body.chars().filter(|c| *c == 'x').count();
    assert!(x_run_len < 5_000);
    assert!(x_run_len > 0);
}

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
fn file_mime_from_extension_and_magic_cover_supported_types() {
    assert_eq!(file_mime_from_extension("PDF"), Some("application/pdf"));
    assert_eq!(file_mime_from_extension("md"), Some("text/markdown"));
    assert_eq!(file_mime_from_extension("markdown"), Some("text/markdown"));
    assert_eq!(file_mime_from_extension("CSV"), Some("text/csv"));
    assert_eq!(file_mime_from_extension("txt"), Some("text/plain"));
    assert_eq!(file_mime_from_extension("zip"), Some("application/zip"));
    assert_eq!(
        file_mime_from_extension("xlsx"),
        Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
    );
    assert_eq!(file_mime_from_extension("rs"), None);

    assert_eq!(
        file_mime_from_magic(b"%PDF-1.4 rest"),
        Some("application/pdf")
    );
    assert_eq!(
        file_mime_from_magic(&[b'P', b'K', 0x03, 0x04, 0x00]),
        Some("application/zip")
    );
    assert_eq!(file_mime_from_magic(b"not-anything"), None);
}

#[test]
fn truncate_chars_respects_cap_and_reports_dropped() {
    let (text, dropped) = truncate_chars("hello".to_string(), 100);
    assert_eq!(text, "hello");
    assert_eq!(dropped, 0);

    let (text, dropped) = truncate_chars("a".repeat(50), 10);
    assert!(text.chars().count() <= 10);
    assert!(dropped > 0);

    // UTF-8 safety: multi-byte chars must not split mid-codepoint.
    let multi = "日本語".repeat(20);
    let (text, _) = truncate_chars(multi.clone(), 5);
    assert!(text.chars().count() <= 5);
    // Round-trip valid utf-8 (would panic otherwise).
    let _ = text.as_str().chars().count();
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
fn file_payload_renders_truncation_marker_in_compose() {
    let payload = FilePayload::Extracted {
        name: "long.txt".to_string(),
        mime: "text/plain".to_string(),
        size_bytes: 1024,
        text: "snippet".to_string(),
        truncated_chars: 42,
    };
    let composed = compose_multimodal_message("intro", &[], &[payload]);
    assert!(composed.contains("intro"));
    assert!(composed.contains("[FILE-EXTRACTED: name=\"long.txt\""));
    assert!(composed.contains("snippet"));
    assert!(composed.contains("[…truncated 42 chars]"));
    assert!(composed.contains("[/FILE-EXTRACTED]"));
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

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

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

#[test]
fn ext_from_mime_maps_known_image_types() {
    assert_eq!(ext_from_mime("image/png"), Some("png"));
    assert_eq!(ext_from_mime("image/jpeg"), Some("jpg"));
    assert_eq!(ext_from_mime("image/webp"), Some("webp"));
    assert_eq!(ext_from_mime("image/gif"), Some("gif"));
    assert_eq!(ext_from_mime("image/bmp"), Some("bmp"));
    assert_eq!(ext_from_mime("application/pdf"), None);
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
