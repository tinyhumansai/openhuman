use super::super::context::{
    build_memory_context, conversation_memory_key, ChannelRuntimeContext,
    CHANNEL_MESSAGE_TIMEOUT_SECS,
};
use super::super::runtime::process_channel_message;
use super::super::{traits, Channel};
use super::common::{HistoryCaptureProvider, NoopMemory, RecordingChannel};
use crate::openhuman::memory::{embeddings::NoopEmbedding, Memory, MemoryCategory, UnifiedMemory};
use crate::openhuman::providers;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn conversation_memory_key_uses_message_id() {
    let msg = traits::ChannelMessage {
        id: "msg_abc123".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "hello".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
    };

    assert_eq!(conversation_memory_key(&msg), "slack_U123_msg_abc123");
}

#[test]
fn conversation_memory_key_is_unique_per_message() {
    let msg1 = traits::ChannelMessage {
        id: "msg_1".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "first".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
    };
    let msg2 = traits::ChannelMessage {
        id: "msg_2".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "second".into(),
        channel: "slack".into(),
        timestamp: 2,
        thread_ts: None,
    };

    assert_ne!(
        conversation_memory_key(&msg1),
        conversation_memory_key(&msg2)
    );
}

#[tokio::test]
async fn autosave_keys_preserve_multiple_conversation_facts() {
    let tmp = TempDir::new().unwrap();
    let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let msg1 = traits::ChannelMessage {
        id: "msg_1".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "I'm Paul".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
    };
    let msg2 = traits::ChannelMessage {
        id: "msg_2".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "I'm 45".into(),
        channel: "slack".into(),
        timestamp: 2,
        thread_ts: None,
    };

    mem.store(
        &conversation_memory_key(&msg1),
        &msg1.content,
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();
    mem.store(
        &conversation_memory_key(&msg2),
        &msg2.content,
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();

    assert_eq!(mem.count().await.unwrap(), 2);

    let recalled = mem.recall("45", 5, None).await.unwrap();
    assert!(recalled.iter().any(|entry| entry.content.contains("45")));
}

#[tokio::test]
async fn build_memory_context_includes_recalled_entries() {
    let tmp = TempDir::new().unwrap();
    let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
    mem.store("age_fact", "Age is 45", MemoryCategory::Conversation, None)
        .await
        .unwrap();

    let context = build_memory_context(&mem, "age", 0.0).await;
    assert!(context.contains("[Memory context]"));
    assert!(context.contains("Age is 45"));
}

#[tokio::test]
async fn process_channel_message_restores_per_sender_history_on_follow_ups() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(HistoryCaptureProvider::default());

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: provider_impl.clone(),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        provider_runtime_options: providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
    });

    process_channel_message(
        runtime_ctx.clone(),
        traits::ChannelMessage {
            id: "msg-a".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
        },
    )
    .await;

    process_channel_message(
        runtime_ctx,
        traits::ChannelMessage {
            id: "msg-b".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "follow up".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
        },
    )
    .await;

    let calls = provider_impl
        .calls
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].len(), 2);
    assert_eq!(calls[0][0].0, "system");
    assert_eq!(calls[0][1].0, "user");
    assert_eq!(calls[1].len(), 4);
    assert_eq!(calls[1][0].0, "system");
    assert_eq!(calls[1][1].0, "user");
    assert_eq!(calls[1][2].0, "assistant");
    assert_eq!(calls[1][3].0, "user");
    assert!(calls[1][1].1.contains("hello"));
    assert!(calls[1][2].1.contains("response-1"));
    assert!(calls[1][3].1.contains("follow up"));
}

// ── AIEOS Identity Tests (Issue #168) ─────────────────────────
