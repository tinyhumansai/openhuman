//! Shared channel runtime state and history helpers.
//!
//! Channel turns carry no per-turn memory recall: memory v2 reaches a new
//! session through `context.md`, injected by the session host, and committed
//! turns are ingested from the `ConversationTurnCommitted` bus event.

use crate::agent::tinyagents::TurnModelSource;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tinytools::Tool;

pub(crate) use tinychannels_bus::context::{
    clear_sender_history, compact_sender_history, ChannelRouteSelection, ConversationHistoryMap,
    CHANNEL_MESSAGE_TIMEOUT_SECS, CHANNEL_TYPING_REFRESH_INTERVAL_SECS,
    DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS, DEFAULT_CHANNEL_MAX_BACKOFF_SECS, MAX_CHANNEL_HISTORY,
};

#[cfg(test)]
pub(crate) use tinychannels_bus::context::{
    CHANNEL_HISTORY_COMPACT_CONTENT_CHARS, CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES,
};

#[cfg(test)]
pub(crate) use tinychannels::context::{
    effective_channel_message_timeout_secs, MIN_CHANNEL_MESSAGE_TIMEOUT_SECS,
};

pub(crate) type TurnModelSourceCacheMap = Arc<Mutex<HashMap<String, TurnModelSource>>>;
pub(crate) type RouteSelectionMap = Arc<Mutex<HashMap<String, ChannelRouteSelection>>>;

#[derive(Clone)]
pub(crate) struct ChannelRuntimeContext {
    /// Explicit durable conversation for a hosted relay; ordinary channels
    /// use their existing per-sender history and have no relay thread.
    pub(crate) relay_thread: Option<String>,
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
    /// The core context this channel runtime was built under. Held weakly: a
    /// channel runtime outlives no tenant. `None` in tests and bespoke hosts
    /// that never bound one.
    pub(crate) core: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
}

impl ChannelRuntimeContext {
    /// The dispatch context this runtime's work runs under.
    pub(crate) fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::from_weak(self.core.as_ref())
    }
}

/// Resolve the configured module client; unit tests use an attested bus fixture.
pub(crate) async fn module_client(ctx: &ChannelRuntimeContext) -> tinybus::module::ModuleClient {
    #[cfg(test)]
    if ctx.config.is_none() {
        return super::module_fixture::client().await;
    }
    crate::modules::client(ctx.config.as_deref().cloned().unwrap_or_default())
}

#[cfg(test)]
pub(crate) fn conversation_history_key(msg: &tinychannels_bus::ChannelMessage) -> String {
    tinychannels::context::conversation_history_key(msg)
}

pub(crate) async fn is_context_window_overflow_error(
    ctx: &ChannelRuntimeContext,
    err: &anyhow::Error,
) -> Result<bool, tinybus::module::ModuleCallError> {
    use tinychannels_bus::module_control::{ChannelControlRequest, ChannelControlResult};
    match module_client(ctx)
        .await
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::PREPARE_CHANNEL_CONTROL,
            (ChannelControlRequest::Overflow {
                message: err.to_string(),
            },),
        )
        .await?
    {
        ChannelControlResult::Overflow(overflow) => Ok(overflow),
        _ => {
            crate::modules::report::malformed_reply("tinychannels");
            Err(tinybus::module::ModuleCallError::ModuleFault)
        }
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
