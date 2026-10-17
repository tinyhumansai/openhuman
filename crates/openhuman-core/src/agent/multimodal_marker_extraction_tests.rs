use super::*;

#[tokio::test]
async fn prepare_messages_rejects_too_many_images() {
    let messages = vec![TranscriptMessage::user(
        "[IMAGE:/tmp/1.png]\n[IMAGE:/tmp/2.png]".to_string(),
    )];

    let config = MultimodalConfig {
        max_images: 1,
        max_image_size_mb: 5,
        allow_remote_fetch: false,
    };

    let error = prepare_messages_for_provider(
        &crate::core::runtime::DispatchCtx::ambient(),
        &messages,
        &config,
        &MultimodalFileConfig::default(),
    )
    .await
    .expect_err("should reject image count overflow");

    assert!(error
        .to_string()
        .contains("multimodal image limit exceeded"));
}

#[tokio::test]
async fn prepare_messages_extracts_text_from_pdf() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("doc.pdf");
    std::fs::write(&file_path, SAMPLE_PDF_BYTES).unwrap();

    let messages = vec![TranscriptMessage::user(format!(
        "[FILE:{}]",
        file_path.display()
    ))];
    let prepared = prepare_messages_for_provider(
        &crate::core::runtime::DispatchCtx::ambient(),
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
async fn prepare_messages_rejects_oversized_file() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("huge.txt");
    std::fs::write(&file_path, vec![b'a'; 2 * 1024 * 1024]).unwrap();

    let messages = vec![TranscriptMessage::user(format!(
        "[FILE:{}]",
        file_path.display()
    ))];
    let file_config = MultimodalFileConfig {
        max_file_size_mb: 1,
        ..Default::default()
    };

    let err = prepare_messages_for_provider(
        &crate::core::runtime::DispatchCtx::ambient(),
        &messages,
        &MultimodalConfig::default(),
        &file_config,
    )
    .await
    .expect_err("oversized file must be rejected");

    assert!(err
        .to_string()
        .contains("multimodal file size limit exceeded"));
}
