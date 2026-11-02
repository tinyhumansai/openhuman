use super::*;
use async_trait::async_trait;
use tinychannels_bus as traits;
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
        relay_thread: None,
        channels_by_name: Arc::new(HashMap::new()),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(model)),
        default_provider: Arc::new("default".into()),
        tools_registry: Arc::new(vec![Box::new(DummyTool) as Box<dyn Tool>]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("prompt"),
        model: Arc::new("model".into()),
        temperature: 0.0,
        max_tool_iterations: 1,
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(PathBuf::from("/tmp")),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    }
}

fn channel_message(channel: &str) -> traits::ChannelMessage {
    traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        channel: channel.into(),
        sender: "alice".into(),
        content: "hello".into(),
        id: "m1".into(),
        reply_target: "reply".into(),
        thread_ts: Some("thread-1".into()),
        timestamp: 0,
        sender_name: None,
    }
}

#[test]
fn timeout_and_history_keys_respect_channel_rules() {
    const { assert!(CHANNEL_MESSAGE_TIMEOUT_SECS >= MIN_CHANNEL_MESSAGE_TIMEOUT_SECS) };

    let telegram = channel_message("telegram");
    let discord = channel_message("discord");
    assert_eq!(conversation_history_key(&telegram), "telegram_alice_reply");
    assert_eq!(
        conversation_history_key(&discord),
        "discord_alice_reply_thread:thread-1"
    );
}

#[tokio::test]
async fn overflow_detection_covers_edge_cases() {
    let ctx = runtime_context();
    assert!(is_context_window_overflow_error(
        &ctx,
        &anyhow::anyhow!("Maximum context length exceeded")
    )
    .await
    .unwrap());
    assert!(
        !is_context_window_overflow_error(&ctx, &anyhow::anyhow!("network timeout"))
            .await
            .unwrap()
    );
}
