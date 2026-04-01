//! Post-turn hook infrastructure for agent self-learning.
//!
//! Hooks fire asynchronously after a turn completes, receiving a snapshot of
//! what happened (user message, assistant response, tool calls with outcomes).
//! The agent does not wait for hooks — they run in the background via `tokio::spawn`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Snapshot of a completed agent turn, passed to every registered hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContext {
    pub user_message: String,
    pub assistant_response: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub turn_duration_ms: u64,
    pub session_id: Option<String>,
    pub iteration_count: usize,
}

/// Record of a single tool invocation within a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub output_snippet: String,
    pub duration_ms: u64,
}

/// Trait for post-turn hooks that react to completed turns.
///
/// Implementations must be cheap to clone (wrapped in `Arc`) and safe to call
/// concurrently from multiple `tokio::spawn` tasks.
#[async_trait]
pub trait PostTurnHook: Send + Sync {
    /// Human-readable name for logging.
    fn name(&self) -> &str;

    /// Called after the agent produces a final response.
    /// Errors are logged but do not propagate to the caller.
    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()>;
}

/// Fire all hooks in parallel, logging errors without blocking the caller.
pub fn fire_hooks(hooks: &[Arc<dyn PostTurnHook>], ctx: TurnContext) {
    for hook in hooks {
        let hook = Arc::clone(hook);
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = hook.on_turn_complete(&ctx).await {
                log::warn!(
                    "[learning] post-turn hook '{}' failed: {e:#}",
                    hook.name()
                );
            }
        });
    }
}
