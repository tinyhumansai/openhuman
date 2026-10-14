//! [`ApprovalSecurityMiddleware`]: route external-effect tool calls through
//! OpenHuman's human-in-the-loop approval gate at the tool boundary.
//!
//! When the caller attaches the session's permission snapshot
//! ([`ApprovalSecurityMiddleware::with_session_permission`]), `requires_approval`
//! first asks whether the permission layer will refuse the call anyway
//! (issue #7649): this wrap sits OUTSIDE `ToolPolicyMiddleware`, so a
//! permission-refused call used to raise an approval card and was denied only
//! after the human answered. A denied verdict skips the prompt — the inner
//! permission layer answers with its own reason instead.

use std::sync::Arc;

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::{ApprovalResolution, ApprovalResolver};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

use crate::security::approval::{
    redact_args, summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome,
};
use tinytools::Tool;

/// `wrap_tool`: route OpenHuman's human-in-the-loop **approval gate** through a
/// named tinyagents tool middleware (issue #4249, Phase 1). A tool with an
/// external effect intercepts through the global [`ApprovalGate`]; a denial
/// short-circuits with the reason as a model-consumable [`TaToolResult`]
/// (`next` is never called), and an allowed call records a terminal audit row
/// once the tool resolves.
///
/// This replaces the inline approval block that used to live in
/// canonical tool dispatch, giving approval a stable middleware name and
/// letting it short-circuit cleanly. Tool-*internal* security (path/command
/// policy via `live_policy`) stays inside each tool — it needs tool-specific
/// operation semantics the harness boundary can't reconstruct generically.
const COMPOSIO_EXECUTE_TOOL: &str = "composio_execute";
const INVALID_COMPOSIO_APPROVAL_NAME: &str = "composio_execute:<invalid-action>";

/// Stable identity used by persistent approval grants.
///
/// `composio_execute` multiplexes every Composio action through one outer tool
/// name. Keying "Always allow" by that name would let approval for one action
/// authorize every later action, so use the namespaced action slug instead.
pub(crate) fn approval_tool_name<'a>(
    tool_name: &'a str,
    args: &'a serde_json::Value,
) -> std::borrow::Cow<'a, str> {
    if tool_name != COMPOSIO_EXECUTE_TOOL {
        return std::borrow::Cow::Borrowed(tool_name);
    }
    let slug = args
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|slug| !slug.is_empty());
    match slug {
        Some(slug) => std::borrow::Cow::Owned(format!("{COMPOSIO_EXECUTE_TOOL}:{slug}")),
        None => std::borrow::Cow::Borrowed(INVALID_COMPOSIO_APPROVAL_NAME),
    }
}

pub(crate) struct ApprovalSecurityMiddleware {
    /// The same `Arc`-shared tool sets the runner registers, used to resolve a
    /// call's OpenHuman `Tool` by name so `external_effect_with_args` can gate.
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    /// The session's channel-permission snapshot and channel name, when the
    /// caller has one (the session chat / live paths pass
    /// `ToolPolicyEnforcement`'s; CLI and sub-agent paths pass `None`).
    ///
    /// `requires_approval` consults it before prompting (issue #7649): this
    /// middleware is the OUTER tool wrap, and the permission ceiling lives on
    /// the inner `ToolPolicyMiddleware` — so without the pre-check a call the
    /// session refuses anyway raises an approval card first and is denied only
    /// after the human answers. A permission-refused call must fail with the
    /// permission layer's own message instead of interrupting anyone.
    permission: Option<(crate::tools::agent_policy::ToolPolicySession, String)>,
}

impl ApprovalSecurityMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(crate) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self {
            tool_sets,
            permission: None,
        }
    }

    /// Attach the session permission boundary the pre-check consults
    /// (`ToolPolicySession` + channel); `None` keeps the prompt-first
    /// behaviour for paths with no session permission snapshot.
    pub(crate) fn with_session_permission(
        mut self,
        permission: Option<(crate::tools::agent_policy::ToolPolicySession, String)>,
    ) -> Self {
        self.permission = permission;
        self
    }

    /// Whether the named tool declares an external effect for these args.
    pub(crate) fn has_external_effect(&self, name: &str, args: &serde_json::Value) -> bool {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .map(|t| t.external_effect_with_args(args))
            .unwrap_or(false)
    }

    pub(crate) async fn requires_approval(&self, name: &str, args: &serde_json::Value) -> bool {
        #[cfg(feature = "modules")]
        let desktop_approval_disabled = match self
            .tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|tool| tool.name() == name)
        {
            Some(tool) => crate::desktop::control::approvals_disabled_for(tool.as_ref()).await,
            None => false,
        };
        #[cfg(not(feature = "modules"))]
        let desktop_approval_disabled = false;
        self.has_external_effect(name, args) && !desktop_approval_disabled
    }
}

/// Host side of the harness approval seam: the tool-set lookup, the global
/// [`ApprovalGate`] hookup and the terminal audit row. The harness
/// `ApprovalGateMiddleware` drives it (see `harness_assembly`).
#[async_trait]
impl ApprovalResolver<crate::agent::tinyagents::host::OpenHumanRunContext>
    for ApprovalSecurityMiddleware
{
    async fn requires_approval(
        &self,
        _ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> bool {
        // Resolve external-effect up front so no tool borrow is held across the
        // approval await.
        let has_ext = self.requires_approval(&call.name, &call.arguments).await;
        if !has_ext {
            tracing::debug!(
                tool = %call.name,
                has_external_effect = false,
                "[tinyagents::mw] checking tool for approval"
            );
            return false;
        }
        // Issue #7649: the session's permission ceiling runs INSIDE this wrap
        // (`ToolPolicyMiddleware::channel_permission_block`), so a call it will
        // refuse reaches that refusal only after this prompt — approval-then-
        // deny. Consult the identical verdict here: a call the session refuses
        // regardless of the human's answer must not raise a card; returning
        // `false` falls through to the permission layer, which denies with its
        // own rendered reason. `false` for `desktop_approval_disabled` is exact:
        // the inherent check above already returned `false` when desktop
        // approvals are disabled, and that is the value the inner
        // `ToolPolicyMiddleware` computes for the same tool on the same config.
        if let Some((session, channel)) = &self.permission {
            let view = super::tool_policy::SessionPermissionView {
                session,
                tool_sets: &self.tool_sets,
                channel,
            };
            if view.channel_permission_block(call, false).is_some() {
                tracing::debug!(
                    tool = %call.name,
                    channel = %channel,
                    "[tinyagents::mw] skipping approval prompt: the session permission \
                     layer will refuse this call"
                );
                return false;
            }
        }
        tracing::debug!(
            tool = %call.name,
            has_external_effect = true,
            "[tinyagents::mw] checking tool for approval"
        );
        true
    }

    async fn resolve(
        &self,
        _ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> ApprovalResolution {
        let Some(gate) = ApprovalGate::try_global() else {
            tracing::warn!(
                tool = %call.name,
                "[tinyagents::mw] approval gate unavailable; external-effect tool will run without interactive approval"
            );
            return ApprovalResolution::Allow { ticket: None };
        };
        let approval_name = approval_tool_name(&call.name, &call.arguments);
        tracing::debug!(
            tool = %call.name,
            approval_name = %approval_name,
            "[tinyagents::mw] routing external-effect tool through approval gate"
        );
        let summary = summarize_action(&call.name, &call.arguments);
        let redacted = redact_args(&call.arguments);
        let (outcome, request_id) = gate
            .intercept_audited(approval_name.as_ref(), &summary, redacted)
            .await;
        match outcome {
            GateOutcome::Deny { reason } => {
                tracing::warn!(
                    tool = %call.name,
                    reason = %reason,
                    "[tinyagents::mw] approval gate denied tool call"
                );
                ApprovalResolution::Deny { reason }
            }
            GateOutcome::Allow => ApprovalResolution::Allow { ticket: request_id },
        }
    }

    /// Record the terminal audit row for an approved external-effect call
    /// (idempotent; a no-op when the id is unknown).
    fn record(&self, ticket: &str, result: &TaToolResult) {
        if let Some(gate) = ApprovalGate::try_global() {
            let exec = if result.is_error {
                ExecutionOutcome::Failure
            } else {
                ExecutionOutcome::Success
            };
            let error = result.is_error.then(|| result.output());
            gate.record_execution(ticket, exec, error.as_deref());
        }
    }
}
