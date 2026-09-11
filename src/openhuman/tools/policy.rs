//! Tool-policy middleware — generic allow/deny gate evaluated before tool execution.
//!
//! The [`ToolPolicy`] trait provides a single extension point for centrally
//! governing which tool invocations proceed. The agent's tool loop calls
//! [`ToolPolicy::evaluate`] before every `tool.execute()`: if the verdict is
//! [`PolicyDecision::Deny`], the tool is never invoked and the denial reason
//! is returned as a `ToolResult::error` to the model.
//!
//! The shipped [`DefaultToolPolicy`] returns `Allow` unconditionally so
//! existing behaviour is preserved. Downstream crates and tests can supply
//! custom policies (rate-limiting, per-tool allow/deny lists, …) by
//! implementing the trait.

use serde_json::Value;

/// Outcome of a policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The tool call may proceed.
    Allow,
    /// The tool call is blocked. The `String` is the human-readable reason
    /// surfaced to the model (and logged).
    Deny(String),
}

/// Trait for tool-execution policies evaluated before every tool invocation.
///
/// Implementations MUST be cheap and synchronous — the policy is called on the
/// agent's hot path. Expensive checks (network, disk) belong in the tool
/// itself or in an async wrapper around this trait.
pub trait ToolPolicy: Send + Sync {
    /// Evaluate whether a tool call is allowed.
    ///
    /// * `tool_name` — the registered name of the tool (`Tool::name()`).
    /// * `args` — the JSON arguments the model supplied for this call.
    fn evaluate(&self, tool_name: &str, args: &Value) -> PolicyDecision;
}

/// Default policy that allows every tool invocation unconditionally.
///
/// This is the backward-compatible default wired into the agent loop when no
/// custom policy is provided.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultToolPolicy;

impl ToolPolicy for DefaultToolPolicy {
    fn evaluate(&self, _tool_name: &str, _args: &Value) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
