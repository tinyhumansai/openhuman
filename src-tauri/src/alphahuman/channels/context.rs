//! Shared channel runtime state and memory helpers.

use crate::alphahuman::memory::Memory;
use crate::alphahuman::observability::Observer;
use crate::alphahuman::providers::{ChatMessage, Provider};
use crate::alphahuman::tools::Tool;
use crate::alphahuman::util::truncate_with_ellipsis;
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
    pub(crate) observer: Arc<dyn Observer>,
    pub(crate) system_prompt: Arc<String>,
    pub(crate) model: Arc<String>,
    pub(crate) temperature: f64,
    pub(crate) auto_save_memory: bool,
    pub(crate) max_tool_iterations: usize,
    pub(crate) min_relevance_score: f64,
    pub(crate) conversation_histories: ConversationHistoryMap,
    pub(crate) provider_cache: ProviderCacheMap,
    pub(crate) route_overrides: RouteSelectionMap,
    pub(crate) api_key: Option<String>,
    pub(crate) api_url: Option<String>,
    pub(crate) reliability: Arc<crate::alphahuman::config::ReliabilityConfig>,
    pub(crate) provider_runtime_options: crate::alphahuman::providers::ProviderRuntimeOptions,
    pub(crate) workspace_dir: Arc<PathBuf>,
    pub(crate) message_timeout_secs: u64,
    pub(crate) multimodal: crate::alphahuman::config::MultimodalConfig,
}

pub(crate) fn conversation_memory_key(msg: &super::traits::ChannelMessage) -> String {
    format!("{}_{}_{}", msg.channel, msg.sender, msg.id)
}

pub(crate) fn conversation_history_key(msg: &super::traits::ChannelMessage) -> String {
    format!("{}_{}", msg.channel, msg.sender)
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
