//! Shared channel runtime state and memory helpers.

use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ChatMessage, Provider};
use crate::openhuman::tools::Tool;
use crate::openhuman::util::truncate_with_ellipsis;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Per-sender conversation history for channel messages.
pub(crate) type ConversationHistoryMap = Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>;
/// Maximum history messages to keep per sender.
pub(crate) const MAX_CHANNEL_HISTORY: usize = 50;

pub(crate) const DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS: u64 = 2;
pub(crate) const DEFAULT_CHANNEL_MAX_BACKOFF_SECS: u64 = 60;
pub(crate) const MIN_CHANNEL_MESSAGE_TIMEOUT_SECS: u64 = 30;
/// Default timeout for processing a single channel message (LLM + tools).
/// Used as fallback when not configured in channels_config.message_timeout_secs.
pub(crate) const CHANNEL_MESSAGE_TIMEOUT_SECS: u64 = 300;
pub(crate) const CHANNEL_PARALLELISM_PER_CHANNEL: usize = 4;
pub(crate) const CHANNEL_MIN_IN_FLIGHT_MESSAGES: usize = 8;
pub(crate) const CHANNEL_MAX_IN_FLIGHT_MESSAGES: usize = 64;
pub(crate) const CHANNEL_TYPING_REFRESH_INTERVAL_SECS: u64 = 4;
pub(crate) const MEMORY_CONTEXT_MAX_ENTRIES: usize = 4;
pub(crate) const MEMORY_CONTEXT_ENTRY_MAX_CHARS: usize = 800;
pub(crate) const MEMORY_CONTEXT_MAX_CHARS: usize = 4_000;
pub(crate) const CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES: usize = 12;
pub(crate) const CHANNEL_HISTORY_COMPACT_CONTENT_CHARS: usize = 600;

pub(crate) type ProviderCacheMap = Arc<Mutex<HashMap<String, Arc<dyn Provider>>>>;
pub(crate) type RouteSelectionMap = Arc<Mutex<HashMap<String, ChannelRouteSelection>>>;

pub(crate) fn effective_channel_message_timeout_secs(configured: u64) -> u64 {
    configured.max(MIN_CHANNEL_MESSAGE_TIMEOUT_SECS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelRouteSelection {
    pub(crate) provider: String,
    pub(crate) model: String,
}

#[derive(Clone)]
pub(crate) struct ChannelRuntimeContext {
    pub(crate) channels_by_name: Arc<HashMap<String, Arc<dyn super::Channel>>>,
    pub(crate) provider: Arc<dyn Provider>,
    pub(crate) default_provider: Arc<String>,
    pub(crate) memory: Arc<dyn Memory>,
    pub(crate) tools_registry: Arc<Vec<Box<dyn Tool>>>,
    pub(crate) system_prompt: Arc<String>,
    pub(crate) model: Arc<String>,
    pub(crate) temperature: f64,
    pub(crate) auto_save_memory: bool,
    pub(crate) max_tool_iterations: usize,
    pub(crate) min_relevance_score: f64,
    pub(crate) conversation_histories: ConversationHistoryMap,
    pub(crate) provider_cache: ProviderCacheMap,
    pub(crate) route_overrides: RouteSelectionMap,
    pub(crate) api_url: Option<String>,
    pub(crate) reliability: Arc<crate::openhuman::config::ReliabilityConfig>,
    pub(crate) provider_runtime_options: crate::openhuman::providers::ProviderRuntimeOptions,
    pub(crate) workspace_dir: Arc<PathBuf>,
    pub(crate) message_timeout_secs: u64,
    pub(crate) multimodal: crate::openhuman::config::MultimodalConfig,
}

pub(crate) fn conversation_memory_key(msg: &super::traits::ChannelMessage) -> String {
    format!("{}_{}_{}", msg.channel, msg.sender, msg.id)
}

pub(crate) fn conversation_history_key(msg: &super::traits::ChannelMessage) -> String {
    let base_key = format!("{}_{}_{}", msg.channel, msg.sender, msg.reply_target);
    // Telegram uses thread_ts as "reply-to message id" for transport targeting.
    // It should not split memory/history into a new conversation per message.
    if msg.channel == "telegram" {
        return base_key;
    }
    if let Some(thread_ts) = msg.thread_ts.as_deref() {
        let thread_ts = thread_ts.trim();
        if !thread_ts.is_empty() {
            return format!("{base_key}_thread:{thread_ts}");
        }
    }
    base_key
}

pub(crate) fn clear_sender_history(ctx: &ChannelRuntimeContext, sender_key: &str) {
    ctx.conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(sender_key);
}

pub(crate) fn compact_sender_history(ctx: &ChannelRuntimeContext, sender_key: &str) -> bool {
    let mut histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let Some(turns) = histories.get_mut(sender_key) else {
        return false;
    };

    if turns.is_empty() {
        return false;
    }

    let keep_from = turns
        .len()
        .saturating_sub(CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES);
    let mut compacted = turns[keep_from..].to_vec();

    for turn in &mut compacted {
        if turn.content.chars().count() > CHANNEL_HISTORY_COMPACT_CONTENT_CHARS {
            turn.content =
                truncate_with_ellipsis(&turn.content, CHANNEL_HISTORY_COMPACT_CONTENT_CHARS);
        }
    }

    *turns = compacted;
    true
}

pub(crate) fn should_skip_memory_context_entry(key: &str, content: &str) -> bool {
    if key.trim().to_ascii_lowercase().ends_with("_history") {
        return true;
    }

    content.chars().count() > MEMORY_CONTEXT_MAX_CHARS
}

pub(crate) fn is_context_window_overflow_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    [
        "exceeds the context window",
        "context window of this model",
        "maximum context length",
        "context length exceeded",
        "too many tokens",
        "token limit exceeded",
        "prompt is too long",
        "input is too long",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
}

pub(crate) async fn build_memory_context(
    mem: &dyn Memory,
    user_msg: &str,
    min_relevance_score: f64,
) -> String {
    let mut context = String::new();

    if let Ok(entries) = mem.recall(user_msg, 5, None).await {
        let mut included = 0usize;
        let mut used_chars = 0usize;

        for entry in entries.iter().filter(|e| match e.score {
            Some(score) => score >= min_relevance_score,
            None => true, // keep entries without a score (e.g. non-vector backends)
        }) {
            if included >= MEMORY_CONTEXT_MAX_ENTRIES {
                break;
            }

            if should_skip_memory_context_entry(&entry.key, &entry.content) {
                continue;
            }

            let content = if entry.content.chars().count() > MEMORY_CONTEXT_ENTRY_MAX_CHARS {
                truncate_with_ellipsis(&entry.content, MEMORY_CONTEXT_ENTRY_MAX_CHARS)
            } else {
                entry.content.clone()
            };

            let line = format!("- {}: {}\n", entry.key, content);
            let line_chars = line.chars().count();
            if used_chars + line_chars > MEMORY_CONTEXT_MAX_CHARS {
                break;
            }

            if included == 0 {
                context.push_str("[Memory context]\n");
            }

            context.push_str(&line);
            used_chars += line_chars;
            included += 1;
        }

        if included > 0 {
            context.push('\n');
        }
    }

    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::channels::traits;
    use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
    use crate::openhuman::providers::Provider;
    use crate::openhuman::tools::{Tool, ToolResult};
    use async_trait::async_trait;

    struct DummyProvider;

    #[async_trait]
    impl Provider for DummyProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("ok".into())
        }
    }

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

    struct MockMemory {
        entries: Vec<MemoryEntry>,
    }

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }

        async fn store(
            &self,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(self.entries.clone())
        }

        async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(self.entries.len())
        }

        async fn health_check(&self) -> bool {
            true
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
        }
    }

    fn runtime_context() -> ChannelRuntimeContext {
        ChannelRuntimeContext {
            channels_by_name: Arc::new(HashMap::new()),
            provider: Arc::new(DummyProvider),
            default_provider: Arc::new("default".into()),
            memory: Arc::new(MockMemory {
                entries: Vec::new(),
            }),
            tools_registry: Arc::new(vec![Box::new(DummyTool) as Box<dyn Tool>]),
            system_prompt: Arc::new("prompt".into()),
            model: Arc::new("model".into()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 1,
            min_relevance_score: 0.4,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_url: None,
            reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
            provider_runtime_options: crate::openhuman::providers::ProviderRuntimeOptions::default(
            ),
            workspace_dir: Arc::new(PathBuf::from("/tmp")),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            multimodal: crate::openhuman::config::MultimodalConfig::default(),
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
        history.push(crate::openhuman::providers::ChatMessage::user("short"));
        history.extend(
            (0..20).map(|idx| {
                crate::openhuman::providers::ChatMessage::assistant("x".repeat(700 + idx))
            }),
        );
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
        let mem = MockMemory {
            entries: vec![
                memory_entry("keep", "v", Some(0.9)),
                memory_entry("drop_history", "ignored", Some(0.9)),
                memory_entry("low", "too low", Some(0.1)),
                memory_entry(
                    "long",
                    &"x".repeat(MEMORY_CONTEXT_ENTRY_MAX_CHARS + 50),
                    Some(0.9),
                ),
            ],
        };

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
        let mem = MockMemory { entries };

        let rendered = build_memory_context(&mem, "hello", 0.4).await;
        assert!(rendered.chars().count() <= MEMORY_CONTEXT_MAX_CHARS + 32);
        assert!(rendered.matches("- k").count() <= MEMORY_CONTEXT_MAX_ENTRIES);
    }
}
