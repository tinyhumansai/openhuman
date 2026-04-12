//! Post-turn hook infrastructure for agent self-learning.
//!
//! Hooks fire asynchronously after a turn completes, receiving a snapshot of
//! what happened (user message, assistant response, tool calls with outcomes).
//! The agent does not wait for hooks — they run in the background via `tokio::spawn`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Snapshot of a completed agent turn, passed to every registered hook.
///
/// This struct captures the full state of the interaction after the LLM has
/// produced a final response, including any intermediate tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContext {
    /// The original message sent by the user.
    pub user_message: String,
    /// The final response emitted by the assistant.
    pub assistant_response: String,
    /// Records of all tools executed during the turn's tool-call loop.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Total wall-clock time the turn took to resolve (ms).
    pub turn_duration_ms: u64,
    /// Optional session identifier for tracking across multiple turns.
    pub session_id: Option<String>,
    /// How many times the LLM was called during this turn.
    pub iteration_count: usize,
}

/// Record of a single tool invocation within a turn.
///
/// Captures the specific inputs and the high-level outcome of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// The name of the tool that was called.
    pub name: String,
    /// The arguments passed to the tool.
    pub arguments: serde_json::Value,
    /// Whether the tool execution reported success.
    pub success: bool,
    /// Sanitized, non-sensitive summary (tool type, status/error class, safe message).
    /// Never contains raw tool output or PII.
    pub output_summary: String,
    /// Duration of the specific tool execution (ms).
    pub duration_ms: u64,
}

/// Produce a safe, non-sensitive summary of a tool result for learning records.
///
/// Strips raw payloads, file contents, API responses, and credentials — returns
/// only the tool name, status, error class (if failed), and a short length hint.
pub fn sanitize_tool_output(output: &str, tool_name: &str, success: bool) -> String {
    if success {
        let char_count = output.chars().count();
        return format!("{tool_name}: ok ({char_count} chars)");
    }

    // For failures, extract a safe error class without raw payload
    let lower = output.to_lowercase();
    let error_class = if lower.contains("timeout") {
        "timeout"
    } else if lower.contains("not found") || lower.contains("no such file") {
        "not_found"
    } else if lower.contains("permission") || lower.contains("denied") {
        "permission_denied"
    } else if lower.contains("connection") || lower.contains("network") {
        "connection_error"
    } else if lower.contains("parse") || lower.contains("invalid") || lower.contains("syntax") {
        "parse_error"
    } else if lower.contains("unknown tool") {
        "unknown_tool"
    } else {
        "error"
    };

    format!("{tool_name}: failed ({error_class})")
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
    log::debug!(
        "[learning] dispatching {} post-turn hook(s) (tool_calls={}, response_chars={})",
        hooks.len(),
        ctx.tool_calls.len(),
        ctx.assistant_response.chars().count()
    );
    for (idx, hook) in hooks.iter().enumerate() {
        let hook = Arc::clone(hook);
        let ctx = ctx.clone();
        log::trace!(
            "[learning] scheduling hook {}/{}: '{}'",
            idx + 1,
            hooks.len(),
            hook.name()
        );
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            match hook.on_turn_complete(&ctx).await {
                Ok(()) => {
                    log::debug!(
                        "[learning] hook '{}' completed in {}ms",
                        hook.name(),
                        started.elapsed().as_millis()
                    );
                }
                Err(e) => {
                    log::warn!(
                        "[learning] hook '{}' failed after {}ms: {e:#}",
                        hook.name(),
                        started.elapsed().as_millis()
                    );
                }
            }
        });
    }
}
