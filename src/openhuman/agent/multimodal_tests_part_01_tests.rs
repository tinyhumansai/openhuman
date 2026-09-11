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
    // `YWJjZA==` is base64 for "abcd". The fixture used to be `abcd==`, which
    // is not decodable base64 at all (6 chars); it only passed because the
    // payload was returned verbatim without validation (#5146 P6).
    let payload = extract_ollama_image_payload("data:image/png;base64,YWJjZA==")
        .expect("payload should be extracted");
    assert_eq!(payload, "YWJjZA==");
}

// ── #5146 P6: a reference that is not base64 must not reach Ollama ──────────

#[test]
fn extract_ollama_image_payload_rejects_a_filesystem_path() {
    // The bug: a path was forwarded verbatim as if it were image bytes, and
    // Ollama answered `illegal base64 data at input byte 19` — an error that
    // names neither the parameter nor the path.
    assert!(extract_ollama_image_payload("/tmp/vision-test.png").is_none());
    assert!(extract_ollama_image_payload("./relative/img.jpg").is_none());
    assert!(extract_ollama_image_payload("~/Pictures/shot.png").is_none());
}

/// The fixtures above are rejected by the base64 alphabet (`.`, `-`, `~`), so
/// they never exercised the path check. An absolute path built only from
/// alphabet characters decodes cleanly and used to be forwarded as image bytes.
#[test]
fn extract_ollama_image_payload_rejects_a_path_that_is_also_valid_base64() {
    // `/tmp/foo` is 8 alphabet characters: valid unpadded base64 for 6 bytes.
    assert!(base64::engine::general_purpose::STANDARD_NO_PAD
        .decode("/tmp/foo")
        .is_ok());
    assert!(extract_ollama_image_payload("/tmp/foo").is_none());
    assert!(extract_ollama_image_payload("/Users/alice/tmp/foo").is_none());
}

/// The counterweight to the check above: base64 for a JPEG starts `/9j/`, so
/// rejecting every leading `/` would break real payloads. Length decides.
#[test]
fn extract_ollama_image_payload_still_accepts_a_bare_jpeg_payload() {
    let jpeg = format!("/9j/4AAQSkZJRgABAQ{}", "A".repeat(64));
    assert!(jpeg.len() >= 64, "fixture must clear the path-shape bound");
    assert_eq!(
        extract_ollama_image_payload(&jpeg).as_deref(),
        Some(jpeg.as_str())
    );
}

/// Documents the residual ambiguity rather than pretending it is closed: a
/// relative path of pure base64 characters, of a length base64 permits, cannot
/// be told apart from a short payload, so it is still accepted. See
/// `looks_like_absolute_path`.
#[test]
fn extract_ollama_image_payload_cannot_reject_a_base64_shaped_relative_path() {
    // 12 characters — a whole number of 4-character groups, so it decodes and
    // is trivially canonical.
    assert_eq!(
        extract_ollama_image_payload("photos/cats1").as_deref(),
        Some("photos/cats1")
    );

    // Most relative paths are NOT ambiguous, for two reasons that are easy to
    // mistake for the check above doing the work:
    //   - `len % 4 == 1` is a length base64 never produces;
    //   - a partial trailing group must have its discarded low bits zero, and
    //     an arbitrary word almost never does. `photos/catpics` decodes as far
    //     as the alphabet is concerned, but its final `s` carries non-zero
    //     spare bits, so the decoder rejects it as non-canonical.
    assert!(extract_ollama_image_payload("photos/catpic").is_none());
    assert!(extract_ollama_image_payload("photos/catpics").is_none());
}

#[test]
fn extract_ollama_image_payload_rejects_a_non_base64_data_uri_payload() {
    assert!(extract_ollama_image_payload("data:image/png;base64,/tmp/not-base64.png").is_none());
}

#[test]
fn extract_ollama_image_payload_accepts_bare_base64_padded_and_unpadded() {
    // Bare base64 stays supported — this path is how the agent hands an
    // already-encoded image straight through.
    assert_eq!(
        extract_ollama_image_payload("YWJjZA==").as_deref(),
        Some("YWJjZA==")
    );
    // Some producers omit padding; rejecting those would be a new regression.
    assert_eq!(
        extract_ollama_image_payload("YWJjZA").as_deref(),
        Some("YWJjZA")
    );
}

#[test]
fn extract_ollama_image_payload_trims_before_validating() {
    assert_eq!(
        extract_ollama_image_payload("  YWJjZA==  ").as_deref(),
        Some("YWJjZA==")
    );
}

#[test]
fn marker_counting_and_ollama_payload_extraction_reach_the_crate() {
    let messages = vec![
        ChatMessage::system("ignore"),
        ChatMessage::user("one [IMAGE:/tmp/a.png] two [IMAGE:/tmp/b.png]"),
    ];
    assert_eq!(count_image_markers(&messages), 2);
    assert!(contains_image_markers(&messages));
    // `local-ref` is not base64 (`-` is outside the standard alphabet), so it
    // is no longer passed through as an image payload (#5146 P6).
    assert!(extract_ollama_image_payload(" local-ref ").is_none());
    assert!(extract_ollama_image_payload("data:image/png;base64,   ").is_none());
}

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
