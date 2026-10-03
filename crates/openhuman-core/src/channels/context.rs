//! Shared channel runtime state and history helpers.
//!
//! Channel turns carry no per-turn memory recall: memory v2 reaches a new
//! session through `context.md`, injected by the session host, and committed
//! turns are ingested from the `ConversationTurnCommitted` bus event.

use crate::agent::tinyagents::TurnModelSource;
use crate::util::truncate_with_ellipsis;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tinyagents_session::transcript::TranscriptMessage;
use tinytools::Tool;

pub(crate) use tinychannels::context::{
    effective_channel_message_timeout_secs, ChannelRouteSelection,
    CHANNEL_HISTORY_COMPACT_CONTENT_CHARS, CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES,
    CHANNEL_MESSAGE_TIMEOUT_SECS, CHANNEL_TYPING_REFRESH_INTERVAL_SECS,
    DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS, DEFAULT_CHANNEL_MAX_BACKOFF_SECS, MAX_CHANNEL_HISTORY,
};

#[cfg(test)]
pub(crate) use tinychannels::context::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS;

/// Per-sender conversation history for channel messages.
pub(crate) type ConversationHistoryMap = Arc<Mutex<HashMap<String, Vec<TranscriptMessage>>>>;

pub(crate) type TurnModelSourceCacheMap = Arc<Mutex<HashMap<String, TurnModelSource>>>;
pub(crate) type RouteSelectionMap = Arc<Mutex<HashMap<String, ChannelRouteSelection>>>;

#[derive(Clone)]
pub(crate) struct ChannelRuntimeContext {
    pub(crate) channels_by_name: Arc<HashMap<String, Arc<dyn super::Channel>>>,
    /// Injected model source used only by tests and bespoke channel hosts.
    /// Production contexts carry `config` and construct crate-native sources.
    pub(crate) turn_model_source: Option<TurnModelSource>,
    pub(crate) default_provider: Arc<String>,
    pub(crate) tools_registry: Arc<Vec<Box<dyn Tool>>>,
    /// Seeds every turn's history. Production uses the refreshing variant so
    /// the active profile and identity-file edits reach the next message
    /// (#6027, #6028); tests pin a fixed literal.
    pub(crate) system_prompt: super::ChannelSystemPrompt,
    pub(crate) model: Arc<String>,
    pub(crate) temperature: f64,
    pub(crate) max_tool_iterations: usize,
    pub(crate) conversation_histories: ConversationHistoryMap,
    pub(crate) turn_model_source_cache: TurnModelSourceCacheMap,
    pub(crate) route_overrides: RouteSelectionMap,
    pub(crate) api_url: Option<String>,
    pub(crate) inference_url: Option<String>,
    pub(crate) reliability: Arc<crate::config::ReliabilityConfig>,
    pub(crate) provider_runtime_options: crate::inference::provider::ProviderRuntimeOptions,
    pub(crate) workspace_dir: Arc<PathBuf>,
    pub(crate) message_timeout_secs: u64,
    pub(crate) multimodal: crate::config::MultimodalConfig,
    pub(crate) multimodal_files: crate::config::MultimodalFileConfig,
    /// Full config for building crate-native turn models (Phase 3 P3-B). `Some` in
    /// production; `None` lets tests inject a model source directly.
    pub(crate) config: Option<Arc<crate::config::Config>>,
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

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
