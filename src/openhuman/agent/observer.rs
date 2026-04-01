//! Legacy observer trait for tool events.
//!
//! Deprecated: prefer the typed `AgentEvent` system in `events.rs`.
//! This module is kept for backward compatibility; use `events::ObserverBridge`
//! to connect legacy observers to the new event stream.

use super::dispatcher::{ParsedToolCall, ToolExecutionResult};

/// Observer for tool events emitted during the agent loop.
///
/// Implementors receive callbacks as tool calls are parsed and executed,
/// enabling real-time event publishing (e.g. to Socket.IO) rather than
/// batch-publishing after the entire loop completes.
///
/// **Deprecated**: Use `AgentEvent` broadcast channel from `events.rs` instead.
pub trait ToolEventObserver: Send + Sync {
    /// Called after tool calls are parsed from the LLM response, before execution.
    fn on_tool_calls(&self, calls: &[ParsedToolCall], round: u32);

    /// Called after all tool calls in a round have been executed.
    fn on_tool_results(&self, results: &[ToolExecutionResult], round: u32);
}
