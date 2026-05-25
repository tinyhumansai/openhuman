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
    /// Backward-compatible mirror of `context.session_id`.
    #[deprecated(note = "use context.session_id")]
    pub session_id: String,
    /// Backward-compatible mirror of `context.channel`.
    #[deprecated(note = "use context.channel")]
    pub channel: String,
    /// Backward-compatible mirror of `context.agent_definition_id`.
    #[deprecated(note = "use context.agent_definition_id")]
    pub agent_definition_id: String,
}

impl fmt::Debug for ToolPolicyRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[allow(deprecated)]
        {
            f.debug_struct("ToolPolicyRequest")
                .field("tool_name", &self.tool_name)
                .field("arguments", &"<redacted>")
                .field("context", &self.context)
                .field("session_id", &redact_for_debug(&self.session_id))
                .field("channel", &redact_for_debug(&self.channel))
                .field("agent_definition_id", &self.agent_definition_id)
                .finish()
        }
    }
}

impl ToolPolicyRequest {
    pub fn new(
        tool_name: impl Into<String>,
        arguments: serde_json::Value,
        context: ToolCallContext,
    ) -> Self {
        #[allow(deprecated)]
        {
            Self {
                tool_name: tool_name.into(),
                arguments,
                session_id: context.session_id.clone(),
                channel: context.channel.clone(),
                agent_definition_id: context.agent_definition_id.clone(),
                context,
            }
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
        let request = ToolPolicyRequest::new(
            "echo",
            serde_json::json!({ "value": 1 }),
            ToolCallContext::session("session", "chat", "orchestrator", "call-1", 1),
        );

        assert_eq!(policy.check(&request).await, ToolPolicyDecision::Allow);
        #[allow(deprecated)]
        {
            assert_eq!(request.session_id, request.context.session_id);
            assert_eq!(request.channel, request.context.channel);
            assert_eq!(
                request.agent_definition_id,
                request.context.agent_definition_id
            );
        }
        assert_eq!(request.context.source, ToolCallSource::Session);
        assert_eq!(request.context.call_id, "call-1");
    }

    #[test]
    fn debug_redacts_sensitive_context_fields() {
        let request = ToolPolicyRequest::new(
            "secrets.lookup",
            serde_json::json!({ "secret": "super-secret-token" }),
            ToolCallContext::session(
                "session-secret-123",
                "private-channel",
                "orchestrator",
                "call-1",
                1,
            ),
        );

        let rendered = format!("{request:?}");
        assert!(rendered.contains("sess..."));
        assert!(rendered.contains("priv..."));
        assert!(!rendered.contains("session-secret-123"));
        assert!(!rendered.contains("private-channel"));
        assert!(!rendered.contains("super-secret-token"));
    }
}
