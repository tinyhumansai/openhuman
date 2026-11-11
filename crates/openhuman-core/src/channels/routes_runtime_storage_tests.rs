use super::*;

#[tokio::test]
async fn handle_runtime_command_telegram_new_reports_unavailable_storage() {
    let tempdir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(tempdir.path().to_path_buf());
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let sender_key = "telegram_alice_chat-remote";

    ctx.conversation_histories.lock().unwrap().insert(
        sender_key.to_string(),
        vec![ChatMessage::user("old history")],
    );

    let msg = inbound(
        "1".into(),
        "alice".into(),
        "chat-remote".into(),
        "/new".into(),
        "telegram".into(),
        Some("42".into()),
    );
    assert!(
        handle_runtime_command_if_needed(
            &ctx,
            &msg,
            Some(&channel),
            &conversation_history_key(&msg)
        )
        .await
    );
    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(
        sent_text(&sent[0]).contains("Created thread"),
        "{}",
        sent_text(&sent[0])
    );
    assert!(sent_text(&sent[0]).contains("failed to persist the chat binding"));
    assert!(sent_text(&sent[0]).contains("host storage is unavailable"));
    assert_eq!(sent[0].thread_ts.as_deref(), Some("42"));
}
