//! [`ApprovalSecurityMiddleware`]: route external-effect tool calls through
//! OpenHuman's human-in-the-loop approval gate at the tool boundary.

use crate::core::runtime::DispatchCtx;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::{
    ApprovalExecutionOutcome, ApprovalResolution, ApprovalResolver,
};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

use crate::security::approval::{summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome};
use tinytools::Tool;

/// Shared fallback latch for hosts that deliberately boot without an
/// `ApprovalGate`. The gate has its own latch when installed; this one ensures
/// a failed ordinary RecordAudit2 write still blocks later resolver instances.
static REQUIRED_AUDIT_FAILURE: AtomicBool = AtomicBool::new(false);

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

struct OrdinaryAuditTicket {
    tool_name: String,
    dispatch: DispatchCtx,
}

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
    /// Tickets for no-prompt external calls. Approval-engine tickets are not
    /// stored here and continue to use RecordExecution2.
    audit_tickets: Mutex<HashMap<String, OrdinaryAuditTicket>>,
}

impl ApprovalSecurityMiddleware {
    /// Build the middleware over the runner's shared tool sets.
    pub(crate) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self {
            tool_sets,
            audit_tickets: Mutex::new(HashMap::new()),
        }
    }

    /// Whether the named tool declares an external effect for these args.
    pub(crate) fn has_external_effect(&self, name: &str, args: &serde_json::Value) -> bool {
        self.resolve_tool(name)
            .map(|tool| tool.external_effect_with_args(args))
            .unwrap_or(false)
    }

    /// The registered tool named `name`, if any.
    pub(crate) fn resolve_tool(&self, name: &str) -> Option<&dyn Tool> {
        self.tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|tool| tool.name() == name)
            .map(|tool| tool.as_ref())
    }

    /// Whether the user turned this desktop tool's approvals off (the desktop
    /// default skips the channel and external-effect approval parks).
    pub(crate) async fn desktop_approvals_disabled(
        &self,
        dispatch: &DispatchCtx,
        name: &str,
    ) -> bool {
        #[cfg(feature = "modules")]
        {
            match self.resolve_tool(name) {
                Some(tool) => crate::desktop::control::approvals_disabled_for(dispatch, tool).await,
                None => false,
            }
        }
        #[cfg(not(feature = "modules"))]
        {
            let _ = (dispatch, name);
            false
        }
    }

    pub(crate) async fn requires_approval(
        &self,
        dispatch: &DispatchCtx,
        name: &str,
        args: &serde_json::Value,
    ) -> bool {
        let external_effect = self.has_external_effect(name, args);
        if !external_effect {
            return false;
        }
        let gate = ApprovalGate::try_global();
        let audit_failed = REQUIRED_AUDIT_FAILURE.load(Ordering::SeqCst)
            || gate
                .as_ref()
                .is_some_and(|gate| gate.audit_failure_latched());
        let approvals_disabled = self.desktop_approvals_disabled(dispatch, name).await;
        Self::requires_resolver(
            external_effect,
            gate.is_some(),
            audit_failed,
            approvals_disabled,
        )
    }

    fn requires_resolver(
        external_effect: bool,
        gate_available: bool,
        audit_failed: bool,
        approvals_disabled: bool,
    ) -> bool {
        external_effect && (!gate_available || audit_failed || !approvals_disabled)
    }
}

impl ApprovalSecurityMiddleware {
    /// [`ApprovalResolver::resolve`] against an explicit gate, with the approval
    /// context the turn's run context carries (chat routing, copilot flag, flow
    /// run). A park is routed to the turn that raised it, never to an ambient one.
    pub(crate) async fn resolve_on(
        &self,
        gate: &ApprovalGate,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> ApprovalResolution {
        let approval_name = approval_tool_name(&call.name, &call.arguments);
        tracing::debug!(
            tool = %call.name,
            approval_name = %approval_name,
            "[tinyagents::mw] routing external-effect tool through approval gate"
        );
        let summary = summarize_action(&call.name, &call.arguments);
        let redacted = crate::security::redact::arguments(&call.arguments);
        let (outcome, request_id) = gate
            .intercept_audited_for_call(
                &ctx.data.approval,
                approval_name.as_ref(),
                &summary,
                redacted,
                (!call.id.trim().is_empty()).then_some(call.id.as_str()),
            )
            .await;
        match outcome {
            GateOutcome::Deny { reason } => {
                tracing::warn!(
                    tool = %call.name,
                    reason = %reason,
                    "[tinyagents::mw] approval gate denied tool call"
                );
                ApprovalResolution::Deny {
                    reason: refused_approval_text(&call.name, &reason),
                }
            }
            GateOutcome::Allow => ApprovalResolution::Allow { ticket: request_id },
        }
    }

    fn resolve_without_gate(tool_name: &str) -> ApprovalResolution {
        tracing::warn!(
            tool = %tool_name,
            "[tinyagents::mw] approval gate unavailable; external-effect tool is refused"
        );
        ApprovalResolution::Deny {
            reason: refused_approval_text(tool_name, "approval gate unavailable"),
        }
    }

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
        // The harness callback cannot fail an already-returned tool result.
        // The gate's required recorder latches future external admissions if
        // this terminal audit write cannot be persisted.
        let _ = gate.record_execution_required(ticket, execution, error.as_deref());
    }

    fn record_external_terminal_with<F>(
        &self,
        gate: Option<&ApprovalGate>,
        event: crate::security::audit_record::HostAuditEvent,
        write: F,
    ) where
        F: FnOnce(crate::security::audit_record::HostAuditEvent) -> Result<(), String>,
    {
        if let Err(error) = write(event) {
            REQUIRED_AUDIT_FAILURE.store(true, Ordering::SeqCst);
            if let Some(gate) = gate {
                gate.latch_audit_failure();
            }
            tracing::error!(
                error = %error,
                "[tinyagents::mw] required external execution audit failed; later external admissions fail closed"
            );
        }
    }

    fn record_terminal_with<F>(
        &self,
        gate: Option<&ApprovalGate>,
        ticket: &str,
        outcome: ApprovalExecutionOutcome<'_>,
        write: F,
    ) where
        F: FnOnce(DispatchCtx, crate::security::audit_record::HostAuditEvent) -> Result<(), String>,
    {
        let ticket_data = self
            .audit_tickets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(ticket);
        if let Some(OrdinaryAuditTicket {
            tool_name,
            dispatch,
        }) = ticket_data
        {
            use ApprovalExecutionOutcome as Outcome;
            let outcome = match outcome {
                Outcome::Result(result) if result.is_error => "failure",
                Outcome::Result(_) => "success",
                Outcome::Failed(_) => "failure",
                Outcome::Cancelled(_) | Outcome::Control(_) => "aborted",
                // A dropped in-flight tool may already have produced effects.
                Outcome::Abandoned => "abandoned",
                _ => "aborted",
            };
            let event = crate::security::audit_record::HostAuditEvent::external_tool_execution(
                &tool_name, outcome,
            );
            self.record_external_terminal_with(gate, event, move |event| write(dispatch, event));
        } else if let Some(gate) = gate {
            Self::record_terminal_on(gate, ticket, outcome);
        }
    }
}

/// What the model is told when the approval prompt for `tool_name` expired
/// unanswered. Like the refusal text it carries no `[policy-denied]` marker
/// (see [`refused_approval_text`]), so the model keeps the turn to tell the user.
fn unanswered_approval_text(tool_name: &str) -> String {
    format!(
        "The approval request for '{tool_name}' was not answered in time, so it was not run. \
         Tell the user the approval window expired and that they must ask again to retry. \
         Do not retry this call yourself this turn and do not achieve the same result another \
         way (shell, CLI, another tool)."
    )
}

/// What the model is told when the approval flow refused `tool_name`.
///
/// A refusal names no human and no timeout, so every refusal reads the same, and
/// it rules out other routes explicitly because `shell` never prompts while
/// autonomy is disabled (`[autonomy] enabled = false`, the default) and was used
/// to redo a refused call. An expired prompt is not a refusal: nobody answered, so
/// the model is told the approval window expired and that the user must ask again.
///
/// Neither text carries a `[policy-denied]` marker on purpose. The marker
/// classifies as a policy failure with a zero recovery budget
/// (`middleware::repeated_failure`), which pauses the turn before the model can
/// answer; the model must get this turn to tell the user the action was not done.
pub(crate) fn refused_approval_text(tool_name: &str, gate_reason: &str) -> String {
    if crate::security::approval::is_unanswered_approval_reason(gate_reason) {
        tracing::warn!(
            tool = %tool_name,
            "[tinyagents::mw] approval prompt expired unanswered"
        );
        return unanswered_approval_text(tool_name);
    }
    tracing::warn!(
        tool = %tool_name,
        reason = %gate_reason,
        "[tinyagents::mw] approval flow declined the tool call"
    );
    "This action was refused and must not be performed this turn — do not retry this call and do \
     not achieve the same result another way (shell, CLI, another tool). Tell the user it was not \
     done."
        .to_string()
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
            return Self::resolve_without_gate(&call.name);
        };
        self.resolve_on(&gate, ctx, call).await
    }

    async fn resolve_allowed(
        &self,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> ApprovalResolution {
        if !self.has_external_effect(&call.name, &call.arguments) {
            return ApprovalResolution::Allow { ticket: None };
        }
        let gate = ApprovalGate::try_global();
        if REQUIRED_AUDIT_FAILURE.load(Ordering::SeqCst)
            || gate
                .as_ref()
                .is_some_and(|gate| gate.audit_failure_latched())
        {
            return Self::resolve_without_gate(&call.name);
        }
        let ticket = format!("host-audit:{}", uuid::Uuid::new_v4());
        self.audit_tickets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                ticket.clone(),
                OrdinaryAuditTicket {
                    tool_name: call.name.clone(),
                    dispatch: ctx.data.dispatch_ctx(),
                },
            );
        ApprovalResolution::Allow {
            ticket: Some(ticket),
        }
    }

    /// Record the terminal audit row for an approved external-effect call
    /// (idempotent; a no-op when the id is unknown).
    fn record(&self, ticket: &str, result: &TaToolResult) {
        self.record_terminal(ticket, ApprovalExecutionOutcome::Result(result));
    }

    fn record_terminal(&self, ticket: &str, outcome: ApprovalExecutionOutcome<'_>) {
        let gate = ApprovalGate::try_global();
        self.record_terminal_with(gate.as_deref(), ticket, outcome, move |dispatch, event| {
            crate::storage::block_on_anyhow(async move {
                crate::security::audit_record::record(&dispatch, None, event)
                    .await
                    .map_err(anyhow::Error::msg)
            })
            .map_err(|error| error.to_string())
        });
    }
}

#[cfg(test)]
#[path = "approval_tests.rs"]
mod tests;
