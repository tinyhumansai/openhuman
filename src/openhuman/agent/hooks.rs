//! Post-turn hook infrastructure for agent self-learning.
//!
//! Hooks fire asynchronously after a turn completes, receiving a snapshot of
//! what happened (user message, assistant response, tool calls with outcomes).
//! The agent does not wait for hooks — they run in the background via `tokio::spawn`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Post-turn hooks supplied by a process embedding OpenHuman.
static EMBEDDER_POST_TURN_HOOKS: std::sync::LazyLock<std::sync::Mutex<Vec<Arc<dyn PostTurnHook>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// Register a hook that is copied into subsequently-created agent sessions.
pub fn register_embedder_post_turn_hook(hook: Arc<dyn PostTurnHook>) {
    EMBEDDER_POST_TURN_HOOKS
        .lock()
        .expect("embedder post-turn hooks poisoned")
        .push(hook);
}

/// Replace an embedder post-turn hook by name, or remove it when absent.
///
/// This prevents a host that rebuilds its core from retaining callbacks from a
/// previous configuration in the process-global registry.
pub fn replace_embedder_post_turn_hook(name: &str, hook: Option<Arc<dyn PostTurnHook>>) {
    let mut hooks = EMBEDDER_POST_TURN_HOOKS
        .lock()
        .expect("embedder post-turn hooks poisoned");
    hooks.retain(|registered| registered.name() != name);
    if let Some(hook) = hook {
        hooks.push(hook);
    }
}

/// Snapshot hooks supplied by the embedding host.
pub fn embedder_post_turn_hooks() -> Vec<Arc<dyn PostTurnHook>> {
    EMBEDDER_POST_TURN_HOOKS
        .lock()
        .expect("embedder post-turn hooks poisoned")
        .clone()
}

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
    /// Canonical agent definition id that produced the turn, when known.
    pub agent_id: Option<String>,
    /// Runtime entrypoint/channel that produced the turn, when known.
    pub entrypoint: Option<String>,
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

/// The two tool lifecycle moments OpenHuman exposes to embedding hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolHookEvent {
    /// Immediately before the selected tool executes.
    PreToolUse,
    /// After the tool returns its final normalized outcome.
    PostToolUse,
}

/// Safe metadata supplied to an embedding host's tool hook.
///
/// The post-execution fields (`output`, `error`) carry the tool's *raw* result
/// text, unlike [`ToolCallRecord::output_summary`], which is sanitized for the
/// learning pipeline. A tool hook is a policy seam — a hook asked to redact
/// secrets from a result cannot do it from a summary — so the trade is
/// deliberate, and it is why these fields exist here and not there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHookContext {
    /// The lifecycle moment that raised this notification.
    pub event: ToolHookEvent,
    /// The provider call id for correlating pre- and post-hook records.
    pub call_id: String,
    /// The registered tool name.
    pub tool_name: String,
    /// Arguments after OpenHuman's recovery/normalization middleware.
    pub arguments: serde_json::Value,
    /// Whether a completed tool succeeded; absent before execution.
    pub success: Option<bool>,
    /// Final tool runtime in milliseconds; absent before execution.
    pub duration_ms: Option<u64>,
    /// Raw tool result text; absent before execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Failure text when the tool errored; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Session the call belongs to, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Canonical agent definition id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// What a pre-tool hook decided about a call.
///
/// This is the enrichment that makes an embedder hook a policy lever rather
/// than a tripwire: before it existed the only expressible answers were "fine"
/// and an `Err` that read to the model as a tool crash.
#[derive(Debug, Clone, Default)]
pub enum ToolHookDecision {
    /// Run the tool as requested.
    #[default]
    Proceed,
    /// Run the tool, but with these arguments instead. Used to redact a
    /// secret, pin a flag, or narrow a path before the tool ever sees it.
    ProceedWith(serde_json::Value),
    /// Refuse the call. `reason` is what the model is told.
    Deny(String),
    /// Escalate to the human through the approval gate. `reason` is what the
    /// human is asked about.
    Ask(String),
}

impl ToolHookDecision {
    /// Whether the call is refused outright.
    pub fn is_deny(&self) -> bool {
        matches!(self, ToolHookDecision::Deny(_))
    }
}

/// Embedder callback around every harness tool execution.
///
/// Implement [`before_tool`](ToolHook::before_tool) and
/// [`after_tool`](ToolHook::after_tool) for a plain observer. Override
/// [`before_tool_decision`](ToolHook::before_tool_decision) and
/// [`after_tool_context`](ToolHook::after_tool_context) when the hook needs to
/// rewrite arguments, escalate to the human, or feed text back to the model;
/// the defaults bridge to the simpler pair so existing implementations keep
/// working unchanged.
#[async_trait]
pub trait ToolHook: Send + Sync {
    /// Human-readable hook identifier for diagnostics.
    fn name(&self) -> &str;
    /// Run before a tool. Returning an error vetoes that tool call.
    async fn before_tool(&self, context: &ToolHookContext) -> anyhow::Result<()>;
    /// Observe a completed tool. Errors are logged and never change its result.
    async fn after_tool(&self, context: &ToolHookContext) -> anyhow::Result<()>;

    /// Decide what happens to a call. Defaults to
    /// [`before_tool`](ToolHook::before_tool), mapping its `Err` to a denial.
    async fn before_tool_decision(&self, context: &ToolHookContext) -> ToolHookDecision {
        match self.before_tool(context).await {
            Ok(()) => ToolHookDecision::Proceed,
            Err(error) => ToolHookDecision::Deny(format!("{error:#}")),
        }
    }

    /// Observe a completed tool and optionally append text to its result, which
    /// the model then sees. Defaults to [`after_tool`](ToolHook::after_tool)
    /// with no appended text.
    async fn after_tool_context(&self, context: &ToolHookContext) -> Option<String> {
        if let Err(error) = self.after_tool(context).await {
            log::warn!("[hooks] post-tool hook '{}' failed: {error:#}", self.name());
        }
        None
    }
}

/// Tool hooks supplied by an embedding host.
static EMBEDDER_TOOL_HOOKS: std::sync::LazyLock<std::sync::Mutex<Vec<Arc<dyn ToolHook>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// Register a tool hook for subsequently-created harnesses.
pub fn register_embedder_tool_hook(hook: Arc<dyn ToolHook>) {
    EMBEDDER_TOOL_HOOKS
        .lock()
        .expect("embedder tool hooks poisoned")
        .push(hook);
}

/// Replace an embedder tool hook by name, or remove it when absent.
///
/// This prevents a host that rebuilds its core from retaining callbacks from a
/// previous configuration in the process-global registry.
pub fn replace_embedder_tool_hook(name: &str, hook: Option<Arc<dyn ToolHook>>) {
    let mut hooks = EMBEDDER_TOOL_HOOKS
        .lock()
        .expect("embedder tool hooks poisoned");
    hooks.retain(|registered| registered.name() != name);
    if let Some(hook) = hook {
        hooks.push(hook);
    }
}

/// Snapshot tool hooks supplied by the embedding host.
pub fn embedder_tool_hooks() -> Vec<Arc<dyn ToolHook>> {
    EMBEDDER_TOOL_HOOKS
        .lock()
        .expect("embedder tool hooks poisoned")
        .clone()
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;

/// Fire all hooks in parallel, logging errors without blocking the caller.
pub fn fire_hooks(hooks: &[Arc<dyn PostTurnHook>], ctx: TurnContext) {
    log::debug!(
        "[learning] dispatching {} post-turn hook(s) (tool_calls={}, response_chars={})",
        hooks.len(),
        ctx.tool_calls.len(),
        ctx.assistant_response.chars().count()
    );
    // Capture the ambient CoreContext before detaching: a bare `tokio::spawn`
    // does not inherit the `CURRENT_CONTEXT` task-local, so under a scoped
    // multi-tenant dispatch a detached hook would fall back to the process
    // default context — and anything context-derived inside the hook (the
    // archivist's `active_memory_guard`, goals enrichment) would read and
    // write another tenant's workspace. Re-entering the scope inside the task
    // keeps the hook on the dispatch it belongs to; when there is no scoped
    // context (the desktop's single-tenant path), `current()` already answers
    // the process default and re-scoping it is a no-op.
    let core_ctx = crate::core::runtime::context::CoreContext::current();
    for (idx, hook) in hooks.iter().enumerate() {
        let hook = Arc::clone(hook);
        let ctx = ctx.clone();
        let core_ctx = core_ctx.clone();
        log::trace!(
            "[learning] scheduling hook {}/{}: '{}'",
            idx + 1,
            hooks.len(),
            hook.name()
        );
        tokio::spawn(async move {
            let run = async move {
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
            };
            match core_ctx {
                Some(scope_ctx) => {
                    crate::core::runtime::context::CoreContext::scope(scope_ctx, run).await
                }
                None => run.await,
            }
        });
    }
}
