//! Generic pre-execution policy hook for agent tool calls.
//!
//! The default policy preserves existing behaviour. Callers that need a
//! narrower runtime can install a custom policy through `AgentBuilder` and
//! deny a tool before any side effect reaches the tool implementation.

use async_trait::async_trait;

/// Structured context for a tool call before it reaches the tool
/// implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Entry point that produced a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallSource {
    Session,
    Bus,
    Channel,
    Cron,
    Webhook,
    Unknown,
}

/// Snapshot of the tool call and session context a policy can inspect.
#[derive(Debug, Clone)]
pub struct ToolPolicyRequest {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub context: ToolCallContext,
    /// Backward-compatible mirror of `context.session_id`.
    pub session_id: String,
    /// Backward-compatible mirror of `context.channel`.
    pub channel: String,
    /// Backward-compatible mirror of `context.agent_definition_id`.
    pub agent_definition_id: String,
}

/// Decision returned by a [`ToolPolicy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyDecision {
    Allow,
    Deny { reason: String },
}

impl ToolPolicyDecision {
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }
}

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
mod tests {
    use super::*;

    #[tokio::test]
    async fn allow_all_policy_allows_every_call() {
        let policy = AllowAllToolPolicy;
        let request = ToolPolicyRequest {
            tool_name: "echo".into(),
            arguments: serde_json::json!({ "value": 1 }),
            context: ToolCallContext::session("session", "chat", "orchestrator", "call-1", 1),
            session_id: "session".into(),
            channel: "chat".into(),
            agent_definition_id: "orchestrator".into(),
        };

        assert_eq!(policy.check(&request).await, ToolPolicyDecision::Allow);
        assert_eq!(request.context.source, ToolCallSource::Session);
        assert_eq!(request.context.call_id, "call-1");
    }
}
