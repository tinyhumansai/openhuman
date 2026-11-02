use super::super::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use super::super::runtime::process_channel_message;
use super::super::runtime::test_support::{run_dispatch_harness, DispatchHarnessOptions};
use super::common::{use_real_agent_handler, HistoryCaptureModel};
use crate::channels::runtime::ChannelHandle;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tinychannels_bus as traits;

#[tokio::test]
async fn dispatch_publishes_tinychannels_inbound_envelope() {
    let observation = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "telegram".to_string(),
        thread_ts: Some("topic-99".to_string()),
        ..Default::default()
    })
    .await;

    let envelope = observation
        .received_event_envelope
        .expect("dispatch should publish inbound envelope");
    assert_eq!(envelope.channel.id, "telegram");
    assert_eq!(envelope.message_id, "m1");
    assert_eq!(envelope.conversation.id, "reply");
    assert_eq!(envelope.conversation.thread_id, None);
    assert_eq!(envelope.conversation.topic_id.as_deref(), Some("topic-99"));
    assert_eq!(envelope.sender.id, "alice");
}

#[tokio::test]
async fn dispatch_preserves_supplied_tinychannels_inbound_envelope() {
    let supplied = tinychannels::ChannelInboundEnvelope {
        channel: tinychannels::channel::ChannelRef {
            id: "slack".into(),
            account_id: Some("bot-a".into()),
        },
        message_id: "m1".into(),
        conversation: tinychannels::channel::ConversationRef {
            kind: tinychannels::channel::ConversationKind::Channel,
            id: "reply".into(),
            scope_id: Some("T123".into()),
            parent_id: None,
            thread_id: Some("thread-1".into()),
            topic_id: None,
        },
        sender: tinychannels::channel::SenderRef {
            id: "alice".into(),
            name: Some("Alice".into()),
            is_bot: false,
            ..Default::default()
        },
        text: "hello".into(),
        ..Default::default()
    };

    let observation = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "slack".to_string(),
        thread_ts: Some("thread-1".to_string()),
        inbound_envelope: Some(supplied),
        ..Default::default()
    })
    .await;

    let envelope = observation
        .received_event_envelope
        .expect("dispatch should publish supplied inbound envelope");
    assert_eq!(envelope.conversation.id, "reply");
    assert_eq!(envelope.channel.account_id.as_deref(), Some("bot-a"));
    assert_eq!(envelope.conversation.scope_id.as_deref(), Some("T123"));
    assert_eq!(envelope.conversation.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(envelope.sender.name.as_deref(), Some("Alice"));
}

#[tokio::test]
async fn host_dispatch_preserves_thread_reply_and_reaction_targets() {
    let threaded = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "telegram".into(),
        thread_ts: Some("topic-99".into()),
        response_text: Some("thread reply".into()),
        ..Default::default()
    })
    .await;
    assert_eq!(threaded.sends.len(), 1);
    assert_eq!(threaded.sends[0].recipient, "reply");
    assert_eq!(threaded.sends[0].thread_ts.as_deref(), Some("topic-99"));
    assert_eq!(threaded.model_calls.len(), 1);
    assert!(threaded.model_calls[0].text.contains("hello"));

    let unthreaded = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "telegram".into(),
        thread_ts: None,
        response_text: Some("plain reply".into()),
        ..Default::default()
    })
    .await;
    assert_eq!(unthreaded.sends.len(), 1);
    assert_eq!(unthreaded.sends[0].thread_ts, None);
    assert_eq!(unthreaded.model_calls.len(), 1);
    assert!(unthreaded.model_calls[0].text.contains("hello"));

    let reaction = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "telegram".into(),
        thread_ts: Some("message-42".into()),
        supports_reactions: true,
        response_text: Some("[REACTION:👍]".into()),
        ..Default::default()
    })
    .await;
    assert_eq!(
        reaction.sends.len(),
        2,
        "the acknowledgment and model reaction are both sent"
    );
    assert_eq!(
        reaction
            .sends
            .iter()
            .filter(|send| send.content == "[REACTION:👍]")
            .count(),
        1,
        "the model reaction is sent once"
    );
    assert_eq!(
        reaction
            .sends
            .iter()
            .filter(|send| send.content.starts_with("[REACTION:") && send.content != "[REACTION:👍]")
            .count(),
        1,
        "the acknowledgment is a distinct reaction"
    );
    assert!(reaction.sends.iter().all(|send| {
        send.content.starts_with("[REACTION:")
            && send.recipient == "reply"
            && send.thread_ts.as_deref() == Some("message-42")
    }));
    assert_eq!(reaction.model_calls.len(), 1);
    assert!(reaction.model_calls[0].text.contains("hello"));
}

#[tokio::test]
async fn dispatch_harness_default_workspace_does_not_reuse_prior_transcript() {
    let first = run_dispatch_harness(DispatchHarnessOptions {
        content: "first isolated harness turn".into(),
        ..Default::default()
    })
    .await;
    assert_eq!(first.model_calls.len(), 1);
    assert_eq!(
        first.sends.len(),
        1,
        "the first harness turn replies successfully"
    );

    let second = run_dispatch_harness(DispatchHarnessOptions {
        content: "second isolated harness turn".into(),
        ..Default::default()
    })
    .await;
    assert_eq!(second.model_calls.len(), 1);
    assert_eq!(
        second.sends.len(),
        1,
        "the second harness turn replies successfully"
    );
    assert!(second.model_calls[0]
        .text
        .contains("second isolated harness turn"));
    assert!(!second.model_calls[0]
        .text
        .contains("first isolated harness turn"));
}

#[tokio::test]
async fn discord_host_pipeline_dispatches_reply_with_thread_context() {
    let observation = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "discord".into(),
        thread_ts: Some("thread-42".into()),
        response_text: Some("hi from discord".into()),
        ..Default::default()
    })
    .await;

    assert_eq!(observation.sends.len(), 1);
    assert_eq!(observation.sends[0].recipient, "reply");
    assert!(observation.sends[0].content.contains("hi from discord"));
    assert_eq!(observation.sends[0].thread_ts.as_deref(), Some("thread-42"));
    assert_eq!(observation.model_calls.len(), 1);
    assert!(observation.model_calls[0].text.contains("hello"));
}

#[tokio::test]
async fn discord_host_pipeline_keeps_thread_histories_separate() {
    let _bus_guard = use_real_agent_handler().await;
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;
    let workspace = tempfile::tempdir().expect("temporary Discord workspace");
    let channel_state = Arc::new(crate::channels::runtime::TestChannel::new("discord"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let model_state = Arc::new(HistoryCaptureModel::default());
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> = model_state.clone();
    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(HashMap::from([("discord".into(), channel)])),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::clone(&model),
        )),
        default_provider: Arc::new("test-provider".into()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test system"),
        model: Arc::new("test-model".into()),
        temperature: 0.0,
        max_tool_iterations: 3,
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::from([(
            "test-provider".into(),
            crate::agent::tinyagents::TurnModelSource::from_model(model),
        )]))),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(workspace.path().to_path_buf()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    });

    let first = traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "discord-a".into(),
        sender: "user-123".into(),
        reply_target: "channel-456".into(),
        content: "first thread message".into(),
        channel: "discord".into(),
        timestamp: 1,
        thread_ts: Some("thread-A".into()),
        sender_name: None,
    };
    let second = traits::ChannelMessage {
        id: "discord-b".into(),
        content: "second thread message".into(),
        thread_ts: Some("thread-B".into()),
        ..first.clone()
    };
    assert_ne!(
        super::super::context::conversation_history_key(&first),
        super::super::context::conversation_history_key(&second)
    );

    process_channel_message(Arc::clone(&runtime_ctx), first).await;
    process_channel_message(runtime_ctx, second).await;

    {
        let calls = model_state.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[0].iter().any(|(role, text)| {
                role == "user"
                    && text.contains("You are responding via the \"discord\" channel")
                    && text.ends_with("first thread message")
            }),
            "first model call was: {:#?}",
            calls[0]
        );
        assert!(
            calls[1].iter().any(|(role, text)| {
                role == "user"
                    && text.contains("You are responding via the \"discord\" channel")
                    && text.ends_with("second thread message")
            }),
            "second model call was: {:#?}",
            calls[1]
        );
        assert!(!calls[1]
            .iter()
            .any(|(role, text)| role == "user" && text.contains("first thread message")));
    }
    assert_eq!(channel_state.sent_messages.lock().await.len(), 2);
}
