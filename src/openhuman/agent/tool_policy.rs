//! Generic pre-execution policy hook for agent tool calls.
//!
//! The default policy preserves existing behaviour. Callers that need a
//! narrower runtime can install a custom policy through `AgentBuilder` and
//! deny a tool before any side effect reaches the tool implementation.

use async_trait::async_trait;

/// Snapshot of the tool call and session context a policy can inspect.
#[derive(Debug, Clone)]
pub struct ToolPolicyRequest {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub session_id: String,
    pub channel: String,
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
            session_id: "session".into(),
            channel: "chat".into(),
            agent_definition_id: "orchestrator".into(),
        };

        assert_eq!(policy.check(&request).await, ToolPolicyDecision::Allow);
    }
}
