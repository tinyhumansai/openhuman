use anyhow::Result;
use openhuman_core::openhuman::agent::multimodal::{
    contains_image_markers, count_image_markers, extract_ollama_image_payload,
    parse_image_markers, prepare_messages_for_provider,
};
use openhuman_core::openhuman::config::MultimodalConfig;
use openhuman_core::openhuman::providers::ChatMessage;

#[test]
fn marker_helpers_cover_mixed_content_and_payload_extraction() {
    let messages = vec![
        ChatMessage::assistant("[IMAGE:/tmp/ignored.png]"),
        ChatMessage::user("look [IMAGE:/tmp/a.png] then [IMAGE:data:image/png;base64,abcd]"),
    ];

    let (cleaned, refs) = parse_image_markers(messages[1].content.as_str());
    assert_eq!(cleaned, "look  then");
    assert_eq!(refs.len(), 2);
    assert_eq!(count_image_markers(&messages), 2);
    assert!(contains_image_markers(&messages));
    assert_eq!(
        extract_ollama_image_payload("data:image/png;base64,abcd").as_deref(),
        Some("abcd")
    );
    assert_eq!(
        extract_ollama_image_payload(" /tmp/a.png ").as_deref(),
        Some("/tmp/a.png")
    );
    let (cleaned_unclosed, refs_unclosed) = parse_image_markers("broken [IMAGE:/tmp/a.png");
    assert_eq!(cleaned_unclosed, "broken [IMAGE:/tmp/a.png");
    assert!(refs_unclosed.is_empty());

    let (cleaned_empty, refs_empty) = parse_image_markers("keep [IMAGE:] literal");
    assert_eq!(cleaned_empty, "keep [IMAGE:] literal");
    assert!(refs_empty.is_empty());

    assert!(!contains_image_markers(&[ChatMessage::assistant("no user refs")]));
}

#[tokio::test]
async fn prepare_messages_passthrough_when_no_user_images_exist() -> Result<()> {
    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::assistant("[IMAGE:/tmp/not-counted.png]"),
        ChatMessage::user("plain text"),
    ];

    let prepared = prepare_messages_for_provider(&messages, &MultimodalConfig::default()).await?;
    assert!(!prepared.contains_images);
    assert_eq!(prepared.messages.len(), 3);
    assert_eq!(prepared.messages[2].content, "plain text");
    Ok(())
}

#[tokio::test]
async fn prepare_messages_accepts_data_uris_and_preserves_other_messages() -> Result<()> {
    let messages = vec![
        ChatMessage::assistant("already there"),
        ChatMessage::user("inspect [IMAGE:data:image/PNG;base64,iVBORw0KGgo=]"),
    ];

    let prepared = prepare_messages_for_provider(&messages, &MultimodalConfig::default()).await?;
    assert!(prepared.contains_images);
    assert_eq!(prepared.messages[0].content, "already there");

    let (cleaned, refs) = parse_image_markers(&prepared.messages[1].content);
    assert_eq!(cleaned, "inspect");
    assert_eq!(refs.len(), 1);
    assert!(refs[0].starts_with("data:image/png;base64,"));
    Ok(())
}

#[tokio::test]
async fn prepare_messages_rejects_invalid_data_uri_forms() {
    let invalid_non_base64 = vec![ChatMessage::user("bad [IMAGE:data:image/png,abcd]")];
    let err = prepare_messages_for_provider(&invalid_non_base64, &MultimodalConfig::default())
        .await
        .expect_err("non-base64 data uri should fail");
    assert!(err.to_string().contains("only base64 data URIs are supported"));

    let invalid_mime = vec![ChatMessage::user("bad [IMAGE:data:text/plain;base64,YQ==]")];
    let err = prepare_messages_for_provider(&invalid_mime, &MultimodalConfig::default())
        .await
        .expect_err("unsupported mime should fail");
    assert!(err.to_string().contains("MIME type is not allowed"));

    let invalid_base64 = vec![ChatMessage::user("bad [IMAGE:data:image/png;base64,%%%]")];
    let err = prepare_messages_for_provider(&invalid_base64, &MultimodalConfig::default())
        .await
        .expect_err("invalid base64 should fail");
    assert!(err.to_string().contains("invalid base64 payload"));
}

#[tokio::test]
async fn prepare_messages_rejects_unknown_local_mime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let file_path = temp.path().join("sample.txt");
    std::fs::write(&file_path, b"not an image").expect("write sample");

    let messages = vec![ChatMessage::user(format!("bad [IMAGE:{}]", file_path.display()))];
    let err = prepare_messages_for_provider(&messages, &MultimodalConfig::default())
        .await
        .expect_err("unknown mime should fail");
    assert!(err.to_string().contains("unknown"));
}
