//! [`ApprovalSecurityMiddleware`]: route external-effect tool calls through
//! OpenHuman's human-in-the-loop approval gate at the tool boundary.

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::{
    ApprovalExecutionOutcome, ApprovalResolution, ApprovalResolver,
};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

use crate::security::approval::{summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome};
use tinytools::Tool;

pub(super) fn approval_resolution_without_gate(
    state: Option<crate::security::approval::gate::ApprovalGateBootState>,
) -> ApprovalResolution {
    if state.is_some_and(|state| state.disabled_by_env) {
        ApprovalResolution::Allow { ticket: None }
    } else {
        ApprovalResolution::Deny {
            reason: "Approval unavailable; external-effect tool was denied.".to_string(),
        }
    }
}

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
}

impl ApprovalSecurityMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(crate) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self { tool_sets }
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

    pub(crate) async fn requires_approval(
        &self,
        dispatch: &DispatchCtx,
        name: &str,
        args: &serde_json::Value,
    ) -> bool {
        #[cfg(feature = "modules")]
        let desktop_approval_disabled = match self
            .tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|tool| tool.name() == name)
        {
            Some(tool) => {
                crate::desktop::control::approvals_disabled_for(dispatch, tool.as_ref()).await
            }
            None => false,
        };
        #[cfg(not(feature = "modules"))]
        let desktop_approval_disabled = false;
        self.has_external_effect(name, args) && !desktop_approval_disabled
    }

    /// Resolve one call against an explicit gate. Tests and embedded runners
    /// use this to preserve the turn's approval routing context without
    /// installing a process-global gate.
    pub(crate) async fn resolve_on(
        &self,
        gate: &ApprovalGate,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> ApprovalResolution {
        let approval_name = approval_tool_name(&call.name, &call.arguments);
        let summary = summarize_action(&call.name, &call.arguments);
        let (outcome, request_id) = gate
            .intercept_audited_for_call(
                &ctx.data.approval,
                approval_name.as_ref(),
                &summary,
                call.arguments.clone(),
                (!call.id.trim().is_empty()).then_some(call.id.as_str()),
            )
            .await;
        match outcome {
            GateOutcome::Deny { reason } => ApprovalResolution::Deny { reason },
            GateOutcome::Allow => ApprovalResolution::Allow { ticket: request_id },
        }
    }

    /// Persist the engine's terminal result through the explicit gate used for
    /// the call. The harness cannot retract a completed tool result, so a
    /// failed required audit write is latched for subsequent admissions.
    pub(crate) fn record_terminal_on(
        gate: &ApprovalGate,
        ticket: &str,
        outcome: ApprovalExecutionOutcome<'_>,
    ) {
        let (execution, error) = match outcome {
            ApprovalExecutionOutcome::Result(result) if result.is_error => {
                (ExecutionOutcome::Failure, Some(result.output()))
            }
            ApprovalExecutionOutcome::Result(_) => (ExecutionOutcome::Success, None),
            ApprovalExecutionOutcome::Failed(_) => (ExecutionOutcome::Failure, None),
            ApprovalExecutionOutcome::Cancelled(_)
            | ApprovalExecutionOutcome::Control(_)
            | ApprovalExecutionOutcome::Abandoned => (ExecutionOutcome::Aborted, None),
            _ => (ExecutionOutcome::Aborted, None),
        };
        let _ = gate.record_execution_required(ticket, execution, error.as_deref());
    }
}

/// Host side of the harness approval seam: the tool-set lookup, the global
/// [`ApprovalGate`] hookup and the terminal audit row. The harness
/// `ApprovalGateMiddleware` drives it (see `turn_middleware`).
#[async_trait]
impl ApprovalResolver<crate::agent::tinyagents::host::OpenHumanRunContext>
    for ApprovalSecurityMiddleware
{
    async fn requires_approval(
        &self,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> bool {
        // Resolve external-effect up front so no tool borrow is held across the
        // approval await.
        let has_ext = self
            .requires_approval(&ctx.data.dispatch_ctx(), &call.name, &call.arguments)
            .await;
        tracing::debug!(
            tool = %call.name,
            has_external_effect = has_ext,
            "[tinyagents::mw] checking tool for approval"
        );
        has_ext
    }

    async fn resolve(
        &self,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> ApprovalResolution {
        let Some(gate) = ApprovalGate::try_global() else {
            let resolution =
                approval_resolution_without_gate(crate::security::approval::gate::try_boot_state());
            if matches!(resolution, ApprovalResolution::Allow { .. }) {
                tracing::info!(
                    tool = %call.name,
                    "[tinyagents::mw] approval gate explicitly disabled by operator"
                );
                return resolution;
            }
            tracing::error!(
                tool = %call.name,
                "[tinyagents::mw] approval gate unavailable; denying external-effect tool"
            );
            return resolution;
        };
        let approval_name = approval_tool_name(&call.name, &call.arguments);
        tracing::debug!(
            tool = %call.name,
            approval_name = %approval_name,
            "[tinyagents::mw] routing external-effect tool through approval gate"
        );
        let summary = summarize_action(&call.name, &call.arguments);
        let (outcome, request_id) = gate
            .intercept_audited(
                &ctx.data.approval,
                approval_name.as_ref(),
                &summary,
                call.arguments.clone(),
            )
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
