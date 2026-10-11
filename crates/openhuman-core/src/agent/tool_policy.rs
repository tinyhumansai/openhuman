//! Generic pre-execution policy hook for agent tool calls.
//!
//! The default policy preserves existing behaviour. Callers that need a
//! narrower runtime can install a custom policy through `AgentBuilder` and
//! deny a tool before any side effect reaches the tool implementation.

use async_trait::async_trait;
use std::fmt;

/// Structured context for a tool call before it reaches the tool
/// implementation.
#[derive(Clone, PartialEq, Eq)]
pub struct ToolCallContext {
    pub session_id: String,
    pub channel: String,
    pub agent_definition_id: String,
    pub call_id: String,
    pub iteration: u32,
    pub source: ToolCallSource,
}

impl ToolCallContext {
    pub fn session(
        session_id: impl Into<String>,
        channel: impl Into<String>,
        agent_definition_id: impl Into<String>,
        call_id: impl Into<String>,
        iteration: u32,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            channel: channel.into(),
            agent_definition_id: agent_definition_id.into(),
            call_id: call_id.into(),
            iteration,
            source: ToolCallSource::Session,
        }
    }
}

impl fmt::Debug for ToolCallContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolCallContext")
            .field("session_id", &redact_for_debug(&self.session_id))
            .field("channel", &redact_for_debug(&self.channel))
            .field("agent_definition_id", &self.agent_definition_id)
            .field("call_id", &self.call_id)
            .field("iteration", &self.iteration)
            .field("source", &self.source)
            .finish()
    }
}

/// Entry point that produced a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Reserved for non-session tool ingress paths wired in follow-up PRs.
pub enum ToolCallSource {
    Session,
    Bus,
    Channel,
    Cron,
    Webhook,
    Unknown,
}

/// Snapshot of the tool call and session context a policy can inspect.
#[derive(Clone)]
pub struct ToolPolicyRequest {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub context: ToolCallContext,
}

impl fmt::Debug for ToolPolicyRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolPolicyRequest")
            .field("tool_name", &self.tool_name)
            .field("arguments", &"<redacted>")
            .field("context", &self.context)
            .finish()
    }
}

impl ToolPolicyRequest {
    pub fn new(
        tool_name: impl Into<String>,
        arguments: serde_json::Value,
        context: ToolCallContext,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            arguments,
            context,
        }
    }
}

fn redact_for_debug(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    let prefix: String = trimmed.chars().take(4).collect();
    format!("{prefix}...")
}

/// Decision returned by a [`ToolPolicy`].
///
/// The vocabulary (allow / deny / require approval) is owned by the harness
/// (`tinyagents_harness::middleware::PolicyDecision`); this alias keeps the
/// host-facing name.
pub type ToolPolicyDecision = tinyagents_harness::middleware::PolicyDecision;

/// Policy middleware invoked before an agent executes a tool.
#[async_trait]
pub trait ToolPolicy: Send + Sync {
    /// Stable policy name for logs and user-visible denial messages.
    fn name(&self) -> &str;

    /// Inspect a tool call and decide whether it can execute.
    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision;
}

/// Default policy used when no caller installs a stricter one.
#[derive(Debug, Default)]
pub struct AllowAllToolPolicy;

#[async_trait]
impl ToolPolicy for AllowAllToolPolicy {
    fn name(&self) -> &str {
        "allow_all"
    }

    async fn check(&self, _request: &ToolPolicyRequest) -> ToolPolicyDecision {
        ToolPolicyDecision::Allow
    }
}

#[cfg(test)]
#[path = "tool_policy_tests.rs"]
mod tests;
