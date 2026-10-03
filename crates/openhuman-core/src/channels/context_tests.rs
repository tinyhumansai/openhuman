use super::*;
use crate::channels::traits;
use async_trait::async_trait;
use tinytools::{Tool, ToolResult};

struct DummyTool;

#[async_trait]
impl Tool for DummyTool {
    fn name(&self) -> &str {
        "dummy"
    }

    fn description(&self) -> &str {
        "dummy"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

fn runtime_context() -> ChannelRuntimeContext {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "ok",
        ]));
    ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(model)),
        default_provider: Arc::new("default".into()),
        tools_registry: Arc::new(vec![Box::new(DummyTool) as Box<dyn Tool>]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("prompt"),
        model: Arc::new("model".into()),
        temperature: 0.0,
        max_tool_iterations: 1,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        provider_runtime_options: crate::inference::provider::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(PathBuf::from("/tmp")),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
    }
}

fn channel_message(channel: &str) -> traits::ChannelMessage {
    traits::ChannelMessage {
        channel: channel.into(),
        sender: "alice".into(),
        content: "hello".into(),
        id: "m1".into(),
        reply_target: "reply".into(),
        thread_ts: Some("thread-1".into()),
        timestamp: 0,
    }
}

#[test]
fn timeout_and_history_keys_respect_channel_rules() {
    assert_eq!(
        effective_channel_message_timeout_secs(10),
        MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
    );
    assert_eq!(effective_channel_message_timeout_secs(120), 120);

    let telegram = channel_message("telegram");
    let discord = channel_message("discord");
    assert_eq!(conversation_history_key(&telegram), "telegram_alice_reply");
    assert_eq!(
        conversation_history_key(&discord),
        "discord_alice_reply_thread:thread-1"
    );
}

#[test]
fn clear_and_compact_sender_history_update_cached_messages() {
    let ctx = runtime_context();
    let sender = "discord_alice_reply_thread:thread-1";
    let mut history = Vec::new();
    history.push(tinyagents_session::transcript::TranscriptMessage::user(
        "short",
    ));
    history.extend((0..20).map(|idx| {
        tinyagents_session::transcript::TranscriptMessage::assistant("x".repeat(700 + idx))
    }));
    ctx.conversation_histories
        .lock()
        .unwrap()
        .insert(sender.into(), history);

    assert!(compact_sender_history(&ctx, sender));
    {
        let compacted = ctx.conversation_histories.lock().unwrap();
        let compacted = compacted.get(sender).unwrap();
        assert_eq!(compacted.len(), CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES);
        assert!(compacted.iter().all(|msg| {
            msg.content.chars().count() <= CHANNEL_HISTORY_COMPACT_CONTENT_CHARS + 3
        }));
    }

    clear_sender_history(&ctx, sender);
    assert!(!ctx
        .conversation_histories
        .lock()
        .unwrap()
        .contains_key(sender));
}

#[test]
fn overflow_detection_covers_edge_cases() {
    assert!(is_context_window_overflow_error(&anyhow::anyhow!(
        "Maximum context length exceeded"
    )));
    assert!(!is_context_window_overflow_error(&anyhow::anyhow!(
        "network timeout"
    )));
}
