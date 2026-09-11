//! Shared channel runtime state and memory helpers.

use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::tinyagents::TurnModelSource;
use crate::openhuman::tools::Tool;
use crate::openhuman::util::truncate_with_ellipsis;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(crate) use tinychannels::context::{
    effective_channel_message_timeout_secs, should_skip_memory_context_entry,
    ChannelRouteSelection, CHANNEL_HISTORY_COMPACT_CONTENT_CHARS,
    CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES, CHANNEL_MESSAGE_TIMEOUT_SECS,
    CHANNEL_TYPING_REFRESH_INTERVAL_SECS, DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS,
    DEFAULT_CHANNEL_MAX_BACKOFF_SECS, MAX_CHANNEL_HISTORY, MEMORY_CONTEXT_ENTRY_MAX_CHARS,
    MEMORY_CONTEXT_MAX_CHARS, MEMORY_CONTEXT_MAX_ENTRIES,
};

#[cfg(test)]
pub(crate) use tinychannels::context::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS;

/// Per-sender conversation history for channel messages.
pub(crate) type ConversationHistoryMap = Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>;

pub(crate) type TurnModelSourceCacheMap = Arc<Mutex<HashMap<String, TurnModelSource>>>;
pub(crate) type RouteSelectionMap = Arc<Mutex<HashMap<String, ChannelRouteSelection>>>;

#[derive(Clone)]
pub(crate) struct ChannelRuntimeContext {
    pub(crate) channels_by_name: Arc<HashMap<String, Arc<dyn super::Channel>>>,
    /// Injected model source used only by tests and bespoke channel hosts.
    /// Production contexts carry `config` and construct crate-native sources.
    pub(crate) turn_model_source: Option<TurnModelSource>,
    pub(crate) default_provider: Arc<String>,
    pub(crate) memory: Arc<crate::openhuman::memory::guard::MemoryGuard>,
    pub(crate) tools_registry: Arc<Vec<Box<dyn Tool>>>,
    /// Seeds every turn's history. Production uses the refreshing variant so
    /// the active profile and identity-file edits reach the next message
    /// (#6027, #6028); tests pin a fixed literal.
    pub(crate) system_prompt: super::ChannelSystemPrompt,
    pub(crate) model: Arc<String>,
    pub(crate) temperature: f64,
    pub(crate) auto_save_memory: bool,
    pub(crate) max_tool_iterations: usize,
    pub(crate) min_relevance_score: f64,
    pub(crate) conversation_histories: ConversationHistoryMap,
    pub(crate) turn_model_source_cache: TurnModelSourceCacheMap,
    pub(crate) route_overrides: RouteSelectionMap,
    pub(crate) api_url: Option<String>,
    pub(crate) inference_url: Option<String>,
    pub(crate) reliability: Arc<crate::openhuman::config::ReliabilityConfig>,
    pub(crate) provider_runtime_options:
        crate::openhuman::inference::provider::ProviderRuntimeOptions,
    pub(crate) workspace_dir: Arc<PathBuf>,
    pub(crate) message_timeout_secs: u64,
    pub(crate) multimodal: crate::openhuman::config::MultimodalConfig,
    pub(crate) multimodal_files: crate::openhuman::config::MultimodalFileConfig,
    /// Full config for building crate-native turn models (Phase 3 P3-B). `Some` in
    /// production; `None` lets tests inject a model source directly.
    pub(crate) config: Option<Arc<crate::openhuman::config::Config>>,
}

pub(crate) fn conversation_memory_key(msg: &super::traits::ChannelMessage) -> String {
    tinychannels::context::conversation_memory_key(msg)
}

pub(crate) fn conversation_history_key(msg: &super::traits::ChannelMessage) -> String {
    tinychannels::context::conversation_history_key(msg)
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

pub(crate) fn is_context_window_overflow_error(err: &anyhow::Error) -> bool {
    tinychannels::context::is_context_window_overflow_message(&err.to_string())
}

use tinymemory_api::provider::MemoryRecall as _;

pub(crate) async fn build_memory_context(
    mem: &crate::openhuman::memory::guard::MemoryGuard,
    user_msg: &str,
    min_relevance_score: f64,
) -> String {
    let mut context = String::new();

    if let Ok(entries) = mem
        .recall(
            user_msg,
            5,
            &tinymemory_api::recall::OwnedRecallOpts::default(),
            // Unrestricted: a channel turn carries no ambient source scope, and
            // the guard narrows against its own allowlist regardless.
            None,
        )
        .await
    {
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
#[path = "context_tests.rs"]
mod tests;
