use super::*;
use crate::openhuman::channels::traits;
use crate::openhuman::tools::{Tool, ToolResult};
use async_trait::async_trait;
use tinymemory_api::types::{MemoryCategory, MemoryEntry};

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

fn memory_entry(key: &str, content: &str, score: Option<f64>) -> MemoryEntry {
    MemoryEntry {
        id: key.into(),
        key: key.into(),
        content: content.into(),
        namespace: None,
        category: MemoryCategory::Conversation,
        timestamp: "now".into(),
        session_id: None,
        score,
        taint: Default::default(),
    }
}

fn runtime_context() -> ChannelRuntimeContext {
    let model: Arc<dyn tinyinference::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "ok",
        ]));
    ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        turn_model_source: Some(
            crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        ),
        default_provider: Arc::new("default".into()),
        memory: crate::openhuman::memory::guard::in_memory::FixedRecallProvider::guarded(Vec::new()),
        tools_registry: Arc::new(vec![Box::new(DummyTool) as Box<dyn Tool>]),
        system_prompt: crate::openhuman::channels::ChannelSystemPrompt::fixed("prompt"),
        model: Arc::new("model".into()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 1,
        min_relevance_score: 0.4,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        provider_runtime_options:
            crate::openhuman::inference::provider::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(PathBuf::from("/tmp")),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
        multimodal_files: crate::openhuman::config::MultimodalFileConfig::default(),
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
    assert_eq!(conversation_memory_key(&telegram), "telegram_alice_m1");
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
    history.push(crate::openhuman::agent::messages::ChatMessage::user(
        "short",
    ));
    history.extend((0..20).map(|idx| {
        crate::openhuman::agent::messages::ChatMessage::assistant("x".repeat(700 + idx))
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
fn skip_and_overflow_detection_cover_edge_cases() {
    assert!(should_skip_memory_context_entry("note_history", "short"));
    assert!(should_skip_memory_context_entry(
        "note",
        &"x".repeat(MEMORY_CONTEXT_MAX_CHARS + 1)
    ));
    assert!(!should_skip_memory_context_entry("note", "short"));

    assert!(is_context_window_overflow_error(&anyhow::anyhow!(
        "Maximum context length exceeded"
    )));
    assert!(!is_context_window_overflow_error(&anyhow::anyhow!(
        "network timeout"
    )));
}

#[tokio::test]
async fn build_memory_context_filters_entries_and_truncates_content() {
    let mem = crate::openhuman::memory::guard::in_memory::FixedRecallProvider::guarded(vec![
        memory_entry("keep", "v", Some(0.9)),
        memory_entry("drop_history", "ignored", Some(0.9)),
        memory_entry("low", "too low", Some(0.1)),
        memory_entry(
            "long",
            &"x".repeat(MEMORY_CONTEXT_ENTRY_MAX_CHARS + 50),
            Some(0.9),
        ),
    ]);

    let rendered = build_memory_context(&mem, "hello", 0.4).await;
    assert!(rendered.starts_with("[Memory context]\n"));
    assert!(rendered.contains("- keep: v"));
    assert!(!rendered.contains("drop_history"));
    assert!(!rendered.contains("too low"));
    assert!(rendered.contains("- long: "));
    assert!(rendered.contains("..."));
}

#[tokio::test]
async fn build_memory_context_honors_total_budget_and_entry_limit() {
    let entries = (0..10)
        .map(|idx| memory_entry(&format!("k{idx}"), &"x".repeat(700), Some(0.9)))
        .collect();
    let mem = crate::openhuman::memory::guard::in_memory::FixedRecallProvider::guarded(entries);

    let rendered = build_memory_context(&mem, "hello", 0.4).await;
    assert!(rendered.chars().count() <= MEMORY_CONTEXT_MAX_CHARS + 32);
    assert!(rendered.matches("- k").count() <= MEMORY_CONTEXT_MAX_ENTRIES);
}
