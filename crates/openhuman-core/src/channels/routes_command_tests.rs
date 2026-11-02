use super::*;

struct RouteRuntimeHostCallback;

#[tinybus::interface(name = "ai.tinyhumans.tinychannels.ChannelsHost")]
impl RouteRuntimeHostCallback {
    async fn dispatch_controller_backend(
        &self,
        _request: tinychannels_bus::ChannelBackendRequest,
    ) -> tinybus::Result<tinychannels_bus::ChannelBackendResult> {
        Err(tinybus::Error::failed("unused route test callback"))
    }
}

async fn open_route_test_runtime() -> crate::modules::channel_runtime::ChannelRuntimeLease {
    let client = crate::channels::module_fixture::client().await;
    crate::modules::channel_runtime::ChannelRuntimeLease::open(client, RouteRuntimeHostCallback)
        .await
        .expect("retain the host callback for remote-control persistence")
}

#[tokio::test]
async fn handle_runtime_command_set_model_clears_sender_history_and_persists_route_override() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let key = "telegram_alice_room";
    ctx.conversation_histories
        .lock()
        .unwrap()
        .insert(key.to_string(), vec![ChatMessage::user("old history")]);
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "room".into(),
        "/model gpt-5-mini".into(),
        "telegram".into(),
        None,
    );

    let handled = handle_runtime_command_if_needed(
        &ctx,
        &msg,
        Some(&channel),
        &conversation_history_key(&msg),
    )
    .await;
    assert!(handled);

    assert!(ctx
        .conversation_histories
        .lock()
        .unwrap()
        .get(key)
        .is_none());
    assert_eq!(
        get_route_selection(&ctx, key),
        ChannelRouteSelection {
            provider: "openai".into(),
            model: "gpt-5-mini".into()
        }
    );

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent_text(&sent[0]).contains("Model switched to `gpt-5-mini`"));
}

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
        "/models".into(),
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
    assert_eq!(sent[0].thread_ts.as_deref(), Some("42"));
}

#[tokio::test]
async fn handle_runtime_command_telegram_new_status_and_sessions_round_trip() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    let _runtime = open_route_test_runtime().await;
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

    let channel: Arc<dyn Channel> = channel_impl.clone();

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
    assert!(
        sent_text(&sent[0]).contains("Started new session"),
        "unexpected /new reply: {}",
        sent_text(&sent[0])
    );
    assert!(sent_text(&sent[0]).contains("In-memory channel history cleared for this chat."));

    assert!(sent_text(&sent[1]).contains("**Status**"));
    assert!(sent_text(&sent[1]).contains("Thread: `Telegram"));
    assert!(sent_text(&sent[1]).contains("Provider: `anthropic`"));
    assert!(sent_text(&sent[1]).contains("Model: `claude-3`"));
    assert!(sent_text(&sent[1]).contains("In-memory turns: 1"));
    assert!(sent_text(&sent[1]).contains("Turn: idle"));

    assert!(sent_text(&sent[2]).contains("**Recent sessions**"));
    assert!(sent_text(&sent[2]).contains("→ `Telegram"));
    assert!(sent
        .iter()
        .all(|message| message.thread_ts.as_deref() == Some("42")));
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
    assert_eq!(sent[0].thread_ts.as_deref(), Some("42"));
}
