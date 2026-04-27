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

    let prepared = prepare_messages_for_provider(&messages, &MultimodalConfig::default())
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

    let error = prepare_messages_for_provider(&messages, &config)
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

    let error = prepare_messages_for_provider(&messages, &MultimodalConfig::default())
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

    let error = prepare_messages_for_provider(&messages, &config)
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

    let composed = compose_multimodal_message("describe", &["data:image/png;base64,abc".into()]);
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
