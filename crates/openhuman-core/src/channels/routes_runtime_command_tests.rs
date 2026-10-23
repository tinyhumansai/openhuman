use super::*;

#[tokio::test]
async fn handle_runtime_command_telegram_status_replies_without_agent() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "chat-remote".into(),
        "/status".into(),
        "telegram".into(),
        Some("42".into()),
    );

    let handled = handle_runtime_command_if_needed(
        &ctx,
        &msg,
        Some(&channel),
        &conversation_history_key(&msg),
    )
    .await;
    assert!(handled);

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(
        sent_text(&sent[0]).contains("**Status**"),
        "status command returned unexpected reply: {:?}",
        sent_text(&sent[0])
    );
    assert!(sent_text(&sent[0]).contains("Provider:"));
}

#[tokio::test]
async fn handle_runtime_command_without_target_channel_still_consumes_command() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "chat-remote".into(),
        "/help".into(),
        "telegram".into(),
        None,
    );

    let handled =
        handle_runtime_command_if_needed(&ctx, &msg, None, &conversation_history_key(&msg)).await;
    assert!(handled);
}

#[tokio::test]
async fn handle_runtime_command_telegram_help_replies_with_remote_command_list() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "chat-remote".into(),
        "/help".into(),
        "telegram".into(),
        Some("42".into()),
    );

    let handled = handle_runtime_command_if_needed(
        &ctx,
        &msg,
        Some(&channel),
        &conversation_history_key(&msg),
    )
    .await;
    assert!(handled);

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent_text(&sent[0]).contains("Remote control:"));
    assert!(sent_text(&sent[0]).contains("`/status`"));
    assert!(sent_text(&sent[0]).contains("`/sessions`"));
    assert!(sent_text(&sent[0]).contains("`/new`"));
    assert!(sent_text(&sent[0]).contains("Model routing: `/model`, `/models`"));
}

#[tokio::test]
async fn handle_runtime_command_telegram_sessions_reports_empty_store() {
    let tempdir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(tempdir.path().to_path_buf());
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "chat-remote".into(),
        "/sessions".into(),
        "telegram".into(),
        Some("42".into()),
    );

    let handled = handle_runtime_command_if_needed(
        &ctx,
        &msg,
        Some(&channel),
        &conversation_history_key(&msg),
    )
    .await;
    assert!(handled);

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent_text(&sent[0]).contains("No conversation threads yet. Send `/new` to create one."));
    assert_eq!(sent[0].thread_id.as_deref(), Some("42"));
}

#[tokio::test]
#[ignore = "the linked tinychannels-module is given no storage backend here, so /new fails to persist the chat binding (host storage is unavailable); needs a storage fixture"]
async fn handle_runtime_command_telegram_new_status_and_sessions_round_trip() {
    let tempdir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(tempdir.path().to_path_buf());
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let sender_key = "telegram_alice_chat-remote";

    ctx.conversation_histories.lock().unwrap().insert(
        sender_key.to_string(),
        vec![ChatMessage::user("old history")],
    );

    let new_msg = inbound(
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
            &new_msg,
            Some(&channel),
            &conversation_history_key(&new_msg)
        )
        .await
    );
    assert!(ctx
        .conversation_histories
        .lock()
        .unwrap()
        .get(sender_key)
        .is_none());

    ctx.conversation_histories
        .lock()
        .unwrap()
        .insert(sender_key.to_string(), vec![ChatMessage::user("after new")]);
    set_route_selection(
        &ctx,
        sender_key,
        ChannelRouteSelection {
            provider: "anthropic".into(),
            model: "claude-3".into(),
        },
    );

    let subscriber = ChannelTurnStateSubscriber::new(tempdir.path().to_path_buf());
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "2".into(),
            sender: "alice".into(),
            reply_target: "chat-remote".into(),
            content: "work".into(),
            thread_ts: Some("42".into()),
            inbound_envelope: None,
            workspace_dir: tempdir.path().to_path_buf(),
        })
        .await;

    let status_msg = inbound(
        "3".into(),
        "alice".into(),
        "chat-remote".into(),
        "/status".into(),
        "telegram".into(),
        Some("42".into()),
    );
    assert!(
        handle_runtime_command_if_needed(
            &ctx,
            &status_msg,
            Some(&channel),
            &conversation_history_key(&status_msg)
        )
        .await
    );

    let sessions_msg = inbound(
        "4".into(),
        "alice".into(),
        "chat-remote".into(),
        "/sessions".into(),
        "telegram".into(),
        Some("42".into()),
    );
    assert!(
        handle_runtime_command_if_needed(
            &ctx,
            &sessions_msg,
            Some(&channel),
            &conversation_history_key(&sessions_msg)
        )
        .await
    );

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert!(sent_text(&sent[0]).contains("Started new session"));
    assert!(sent_text(&sent[0]).contains("In-memory channel history cleared for this chat."));

    assert!(sent_text(&sent[1]).contains("**Status**"));
    assert!(sent_text(&sent[1]).contains("Thread: `Telegram"));
    assert!(sent_text(&sent[1]).contains("Provider: `anthropic`"));
    assert!(sent_text(&sent[1]).contains("Model: `claude-3`"));
    assert!(sent_text(&sent[1]).contains("In-memory turns: 1"));
    assert!(sent_text(&sent[1]).contains("Turn: in progress"));

    assert!(sent_text(&sent[2]).contains("**Recent sessions**"));
    assert!(sent_text(&sent[2]).contains("→ `Telegram"));
    assert!(sent
        .iter()
        .all(|message| message.thread_id.as_deref() == Some("42")));
}

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
    assert_eq!(sent[0].thread_id.as_deref(), Some("42"));
}
