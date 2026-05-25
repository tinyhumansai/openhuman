//! Generic pre-execution policy hook for agent tool calls.
//!
//! The default policy preserves existing behaviour. Callers that need a
//! narrower runtime can install a custom policy through `AgentBuilder` and
//! deny a tool before any side effect reaches the tool implementation.

use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};
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
    pub generated_tool: Option<GeneratedToolRuntimeContext>,
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
                .field("generated_tool", &self.generated_tool)
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
                generated_tool: None,
            }
        }
    }

    pub fn with_generated_tool_context(mut self, context: GeneratedToolRuntimeContext) -> Self {
        self.generated_tool = Some(context);
        self
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
    /// The policy requires an approval handoff before execution.
    ///
    /// Session execution currently treats this as fail-closed through
    /// [`ToolPolicyDecision::blocking_reason`]. Callers that can prompt for
    /// approval may branch on this variant and retry after approval is granted.
    RequireApproval {
        reason: String,
    },
    Deny {
        reason: String,
    },
}

impl ToolPolicyDecision {
    pub fn require_approval(reason: impl Into<String>) -> Self {
        Self::RequireApproval {
            reason: reason.into(),
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    /// Reason used by fail-closed executors that cannot complete approvals inline.
    pub fn blocking_reason(&self) -> Option<&str> {
        match self {
            Self::Allow => None,
            Self::RequireApproval { reason } | Self::Deny { reason } => Some(reason.as_str()),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedToolRuntimeContext {
    pub provider_id: String,
    pub capability_id: String,
    pub risk: GeneratedToolRuntimeRisk,
    pub source_digest: Option<String>,
    pub approval_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GeneratedToolRuntimeRisk {
    Read,
    Write,
    ExternalWrite,
    Execute,
    Dangerous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeToolPolicyAction {
    Allow,
    RequireApproval,
    Deny,
}

#[derive(Debug, Clone, Default)]
pub struct GeneratedToolRuntimePolicyConfig {
    pub enabled: bool,
    pub revoked_providers: BTreeSet<String>,
    pub revoked_capabilities: BTreeSet<String>,
    pub provider_actions: BTreeMap<String, RuntimeToolPolicyAction>,
    pub capability_actions: BTreeMap<String, RuntimeToolPolicyAction>,
    pub risk_actions: BTreeMap<GeneratedToolRuntimeRisk, RuntimeToolPolicyAction>,
}

#[derive(Debug, Clone)]
pub struct GeneratedToolRuntimePolicy {
    config: GeneratedToolRuntimePolicyConfig,
}

impl GeneratedToolRuntimePolicy {
    pub fn new(config: GeneratedToolRuntimePolicyConfig) -> Self {
        Self { config }
    }

    fn action_for(
        &self,
        tool_name: &str,
        context: &GeneratedToolRuntimeContext,
    ) -> (RuntimeToolPolicyAction, String) {
        if self
            .config
            .revoked_providers
            .contains(context.provider_id.as_str())
        {
            tracing::debug!(
                tool = tool_name,
                provider_id = context.provider_id.as_str(),
                capability_id = context.capability_id.as_str(),
                risk = ?context.risk,
                action = ?RuntimeToolPolicyAction::Deny,
                "[generated_tool_runtime] provider revoked"
            );
            return (
                RuntimeToolPolicyAction::Deny,
                format!("provider `{}` is revoked", context.provider_id),
            );
        }
        if self
            .config
            .revoked_capabilities
            .contains(context.capability_id.as_str())
        {
            tracing::debug!(
                tool = tool_name,
                provider_id = context.provider_id.as_str(),
                capability_id = context.capability_id.as_str(),
                risk = ?context.risk,
                action = ?RuntimeToolPolicyAction::Deny,
                "[generated_tool_runtime] capability revoked"
            );
            return (
                RuntimeToolPolicyAction::Deny,
                format!("capability `{}` is revoked", context.capability_id),
            );
        }
        if let Some(action) = self.config.capability_actions.get(&context.capability_id) {
            tracing::debug!(
                tool = tool_name,
                provider_id = context.provider_id.as_str(),
                capability_id = context.capability_id.as_str(),
                risk = ?context.risk,
                action = ?action,
                "[generated_tool_runtime] capability action matched"
            );
            return (
                *action,
                format!(
                    "capability `{}` matched runtime policy",
                    context.capability_id
                ),
            );
        }
        if let Some(action) = self.config.provider_actions.get(&context.provider_id) {
            tracing::debug!(
                tool = tool_name,
                provider_id = context.provider_id.as_str(),
                capability_id = context.capability_id.as_str(),
                risk = ?context.risk,
                action = ?action,
                "[generated_tool_runtime] provider action matched"
            );
            return (
                *action,
                format!("provider `{}` matched runtime policy", context.provider_id),
            );
        }
        if let Some(action) = self.config.risk_actions.get(&context.risk) {
            tracing::debug!(
                tool = tool_name,
                provider_id = context.provider_id.as_str(),
                capability_id = context.capability_id.as_str(),
                risk = ?context.risk,
                action = ?action,
                "[generated_tool_runtime] risk action matched"
            );
            return (
                *action,
                format!("risk `{:?}` matched runtime policy", context.risk),
            );
        }
        tracing::trace!(
            tool = tool_name,
            provider_id = context.provider_id.as_str(),
            capability_id = context.capability_id.as_str(),
            risk = ?context.risk,
            action = ?RuntimeToolPolicyAction::Allow,
            "[generated_tool_runtime] default allow"
        );
        (
            RuntimeToolPolicyAction::Allow,
            format!("tool `{tool_name}` allowed"),
        )
    }
}

#[async_trait]
impl ToolPolicy for GeneratedToolRuntimePolicy {
    fn name(&self) -> &str {
        "generated_tool_runtime"
    }

    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        if !self.config.enabled {
            tracing::trace!(
                policy = self.name(),
                tool = request.tool_name.as_str(),
                "[generated_tool_runtime] policy disabled"
            );
            return ToolPolicyDecision::Allow;
        }
        let Some(context) = request.generated_tool.as_ref() else {
            tracing::trace!(
                policy = self.name(),
                tool = request.tool_name.as_str(),
                "[generated_tool_runtime] context missing"
            );
            return ToolPolicyDecision::Allow;
        };
        let (action, reason) = self.action_for(&request.tool_name, context);
        tracing::debug!(
            policy = self.name(),
            tool = request.tool_name.as_str(),
            provider_id = context.provider_id.as_str(),
            capability_id = context.capability_id.as_str(),
            risk = ?context.risk,
            action = ?action,
            reason = reason.as_str(),
            "[generated_tool_runtime] policy decision"
        );
        match action {
            RuntimeToolPolicyAction::Allow => ToolPolicyDecision::Allow,
            RuntimeToolPolicyAction::RequireApproval => {
                ToolPolicyDecision::require_approval(reason)
            }
            RuntimeToolPolicyAction::Deny => ToolPolicyDecision::deny(reason),
        }
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

    fn generated_request() -> ToolPolicyRequest {
        ToolPolicyRequest::new(
            "email.send",
            serde_json::json!({ "to": "user@example.com" }),
            ToolCallContext::session("session", "chat", "orchestrator", "call-1", 1),
        )
        .with_generated_tool_context(GeneratedToolRuntimeContext {
            provider_id: "mail.runtime".to_string(),
            capability_id: "email.send".to_string(),
            risk: GeneratedToolRuntimeRisk::ExternalWrite,
            source_digest: Some("sha256:abc".to_string()),
            approval_id: None,
        })
    }

    #[tokio::test]
    async fn generated_runtime_policy_allows_when_disabled() {
        let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig::default());

        assert_eq!(
            policy.check(&generated_request()).await,
            ToolPolicyDecision::Allow
        );
    }

    #[tokio::test]
    async fn generated_runtime_policy_allows_when_enabled_but_missing_context() {
        let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
            enabled: true,
            ..Default::default()
        });

        let request = ToolPolicyRequest::new(
            "echo",
            serde_json::json!({ "value": 1 }),
            ToolCallContext::session("session", "chat", "orchestrator", "call-1", 1),
        );

        assert_eq!(policy.check(&request).await, ToolPolicyDecision::Allow);
    }

    #[tokio::test]
    async fn generated_runtime_policy_denies_revoked_provider() {
        let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
            enabled: true,
            revoked_providers: BTreeSet::from(["mail.runtime".to_string()]),
            ..Default::default()
        });

        let decision = policy.check(&generated_request()).await;
        assert!(matches!(decision, ToolPolicyDecision::Deny { .. }));
        assert!(decision.blocking_reason().unwrap().contains("revoked"));
    }

    #[tokio::test]
    async fn generated_runtime_policy_denies_revoked_capability() {
        let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
            enabled: true,
            revoked_capabilities: BTreeSet::from(["email.send".to_string()]),
            ..Default::default()
        });

        let decision = policy.check(&generated_request()).await;
        assert!(matches!(decision, ToolPolicyDecision::Deny { .. }));
        assert!(decision.blocking_reason().unwrap().contains("capability"));
    }

    #[tokio::test]
    async fn generated_runtime_policy_requires_approval_by_risk() {
        let policy = GeneratedToolRuntimePolicy::new(GeneratedToolRuntimePolicyConfig {
            enabled: true,
            risk_actions: BTreeMap::from([(
                GeneratedToolRuntimeRisk::ExternalWrite,
                RuntimeToolPolicyAction::RequireApproval,
            )]),
            ..Default::default()
        });

        let decision = policy.check(&generated_request()).await;
        assert!(matches!(
            decision,
            ToolPolicyDecision::RequireApproval { .. }
        ));
    }
}
