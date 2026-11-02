use super::*;

#[tokio::test]
async fn handle_runtime_command_telegram_new_status_and_sessions_round_trip() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let workspace = linked.config().workspace_dir.clone();
    let mut ctx = runtime_context(workspace.clone());
    let _core =
        crate::channels::runtime::test_support::bind_test_core_context(&mut ctx, linked.config());
    let channel_state = Arc::new(TestChannel::new("recording"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let sender_key = "telegram_alice_chat-remote";

    let new_msg = ChannelMessage {
        reply_to_id: None,
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/new".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        topic_id: Some("42".into()),
        sender_name: None,
    };
    assert!(handle_linked_runtime_command(&mut ctx, &new_msg, Some(&channel), &linked, None).await);
    set_route_selection(
        &ctx,
        sender_key,
        ChannelRouteSelection {
            provider: "anthropic".into(),
            model: "claude-3".into(),
        },
    );

    let subscriber =
        ChannelTurnStateSubscriber::with_objects(workspace.clone(), linked.module_objects());
    let event_core = ctx
        .core
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
        .expect("the linked route commands and event must share a core scope");
    crate::core::runtime::CoreContext::scope(event_core, async {
        subscriber
            .handle(&DomainEvent::ChannelMessageReceived {
                channel: "telegram".into(),
                message_id: "2".into(),
                sender: "alice".into(),
                reply_target: "chat-remote".into(),
                content: "work".into(),
                thread_ts: None,
                inbound_envelope: None,
                workspace_dir: workspace.clone(),
            })
            .await;
    })
    .await;

    let status_msg = ChannelMessage {
        reply_to_id: None,
        id: "3".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/status".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        topic_id: Some("42".into()),
        sender_name: None,
    };
    assert!(
        handle_linked_runtime_command(&mut ctx, &status_msg, Some(&channel), &linked, None).await
    );

    let sessions_msg = ChannelMessage {
        reply_to_id: None,
        id: "4".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/sessions".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        topic_id: Some("42".into()),
        sender_name: None,
    };
    assert!(
        handle_linked_runtime_command(&mut ctx, &sessions_msg, Some(&channel), &linked, None).await
    );

    let sent = channel_state.sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        3,
        "{:?}",
        sent.iter().map(|m| m.content.clone()).collect::<Vec<_>>()
    );
    assert!(sent[0].content.contains("Started new session"));
    assert!(sent[0]
        .content
        .contains("Durable conversation history reset for this chat."));

    assert!(sent[1].content.contains("**Status**"));
    assert!(sent[1].content.contains("Thread: `Telegram"));
    assert!(sent[1].content.contains("Provider: `anthropic`"));
    assert!(sent[1].content.contains("Model: `claude-3`"));
    assert!(sent[1].content.contains("Session turns: 0"));
    assert!(
        sent[1].content.contains("Turn: in progress"),
        "status should observe the received-event state in its host scope: {}",
        sent[1].content
    );

    assert!(sent[2].content.contains("**Recent sessions**"));
    assert!(sent[2].content.contains("→ `Telegram"));
    assert!(sent
        .iter()
        .all(|message| message.topic_id.as_deref() == Some("42")));
}

#[tokio::test]
async fn handle_runtime_command_telegram_new_uses_module_storage() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let workspace = linked.config().workspace_dir.clone();
    let mut ctx = runtime_context(workspace);
    let channels = linked.channels();
    let channel = channels
        .iter()
        .find(|(name, _)| tinychannels_bus::provider_id(name) == "telegram")
        .map(|(_, channel)| Arc::clone(channel))
        .expect("linked runtime should expose its Telegram module handle");
    let msg = ChannelMessage {
        topic_id: Some("42".into()),
        id: "200".into(),
        reply_to_id: Some("100".into()),
        sender: "alice".into(),
        // The module prepares the chat and topic as distinct canonical fields.
        reply_target: "-200".into(),
        content: "/new".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        sender_name: None,
    };
    assert!(
        handle_linked_runtime_command(&mut ctx, &msg, Some(&channel), &linked, Some("-200")).await
    );
    let topic_only_msg = ChannelMessage {
        id: "201".into(),
        reply_to_id: None,
        thread_ts: None,
        ..msg.clone()
    };
    assert!(
        handle_linked_runtime_command(
            &mut ctx,
            &topic_only_msg,
            Some(&channel),
            &linked,
            Some("-200"),
        )
        .await
    );
    let sent = linked
        .provider_request_bodies("/bottoken/sendMessage")
        .await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["chat_id"], "-200");
    assert_eq!(sent[0]["message_thread_id"], "42");
    assert_eq!(sent[0]["reply_to_message_id"], 100);
    assert!(sent[0]["text"]
        .as_str()
        .unwrap()
        .contains("Started new session"));
    assert!(sent[0]["text"]
        .as_str()
        .unwrap()
        .contains("Durable conversation history reset"));
    assert_eq!(sent[1]["chat_id"], "-200");
    assert_eq!(sent[1]["message_thread_id"], "42");
    assert!(sent[1].get("reply_to_message_id").is_none());
}
