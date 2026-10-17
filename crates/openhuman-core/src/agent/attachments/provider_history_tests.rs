use super::tests::Probe;
use super::*;

#[tokio::test]
async fn missing_historical_media_does_not_block_latest_valid_media() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(
        temp.path().join("latest.png"),
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
    )
    .await
    .unwrap();
    let mut config = Config::default();
    config.action_dir = temp.path().into();
    let probe = Arc::new(Probe::default());
    let wrapper = wrap(
        &crate::core::runtime::DispatchCtx::host_default(),
        probe.clone(),
        Arc::new(config),
        "gpt-4o",
        "openai",
    );
    let image = |path: &str| {
        Message::User(tinyinference_llm::message::UserMessage {
            content: vec![ContentBlock::Image(ImageRef {
                url: path.into(),
                mime_type: Some("image/png".into()),
            })],
        })
    };
    let signed_urls = [
        "https://files.example.test/old.png?token=private-value",
        "ftp://files.example.test/old.png?signature=also-private",
    ];
    let mut old_content = signed_urls
        .iter()
        .map(|url| {
            ContentBlock::Image(ImageRef {
                url: (*url).into(),
                mime_type: Some("image/png".into()),
            })
        })
        .collect::<Vec<_>>();
    old_content.extend((0..8).map(|index| {
        ContentBlock::Image(ImageRef {
            url: format!("deleted-prior-upload-{index}.png"),
            mime_type: Some("image/png".into()),
        })
    }));
    let old_user = Message::User(tinyinference_llm::message::UserMessage {
        content: old_content,
    });
    let request = ModelRequest::new(vec![old_user, image("latest.png")]);
    let durable = request.clone();

    assert!(wrapper.invoke(&(), request).await.is_err());
    let captured = probe.requests.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    let history = &captured[0].messages[0];
    assert!(history
        .text()
        .contains("Earlier attachment omitted by history budget"));
    assert!(history.text().contains("Earlier attachment unavailable"));
    assert!(history.text().contains("deleted-prior-upload-0.png"));
    assert!(!serde_json::to_string(history).unwrap().contains("base64"));
    assert!(!serde_json::to_string(history)
        .unwrap()
        .contains("private-value"));
    assert!(!serde_json::to_string(history)
        .unwrap()
        .contains("also-private"));
    let durable_text = serde_json::to_string(&durable).unwrap();
    assert!(signed_urls.iter().all(|url| durable_text.contains(url)));
    assert!(serde_json::to_string(&captured[0].messages[1])
        .unwrap()
        .contains("data:image/png;base64,"));
}

#[cfg(unix)]
#[tokio::test]
async fn secure_open_refuses_replaced_final_symlink() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let allowed = root.join("allowed.txt");
    let outside = root.join("outside.txt");
    tokio::fs::write(&allowed, b"allowed").await.unwrap();
    tokio::fs::write(&outside, b"secret").await.unwrap();
    tokio::fs::remove_file(&allowed).await.unwrap();
    symlink(&outside, &allowed).unwrap();

    let error = secure_open(&allowed, &root).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
}
