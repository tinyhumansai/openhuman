//! Autonomy-tier and approval gating for flow nodes.
//!
//! Every node that reaches outside the process — a tool call, an HTTP request,
//! a code run — passes through here first. The tier decides whether the action
//! is allowed outright, needs a human, or is refused; the approval gate is what
//! actually performs the round-trip when a human is needed.

use serde_json::Value;
use tinyflows::error::{EngineError, Result};

use crate::openhuman::security::{
    CommandClass, GateDecision, SecurityPolicy, POLICY_BLOCKED_MARKER,
};

/// Hard autonomy-tier gate for an *acting* flow node (Phase 2).
///
/// A flow run scopes a `TrustedAutomation { Workflow }` origin, but the acting
/// power of a run is still bounded by the user's `[autonomy]` tier — the same
/// [`SecurityPolicy`] the agent tool-loop honors (`SecurityPolicy::from_config`
/// off the `[autonomy]` block). Before an `http_request` (Network-class) or
/// `code` (Write-class) node dispatches, we consult
/// [`SecurityPolicy::gate_decision`] for that node's [`CommandClass`] and refuse
/// outright when the tier `Block`s it — mirroring how `curl`/`shell` acting
/// tools gate (`policy.gate_decision(CommandClass::Network)`), so a read-only
/// run can never reach the network or run arbitrary code.
///
/// `Allow`/`Prompt` return `Ok(decision)`: this function only enforces the
/// non-negotiable `Block` floor itself. The caller uses the returned
/// [`GateDecision`] to drive [`gate_call_for_tier`] immediately after, which is
/// what actually performs the `Prompt` round-trip (see that function's doc for
/// why this is not automatic — a saved workflow's own `require_approval` flag
/// would otherwise silently override the tier's `Prompt` decision). The error
/// is prefixed with [`POLICY_BLOCKED_MARKER`] so the harness's repeated-failure
/// middleware recognizes it as a permanent, don't-retry refusal.
///
/// `pub(crate)` (not `http_request`/`code`-private): the `memory` node's
/// [`OpenHumanMemory`](super::super::memory_adapter::OpenHumanMemory) adapter
/// reuses this exact function — `CommandClass::Read` for
/// recall/search/flavour/people, `CommandClass::Write` for remember/forget —
/// rather than growing a second permission path for the new node kind.
pub(crate) fn enforce_node_tier_gate(
    security: &SecurityPolicy,
    class: CommandClass,
    node: &str,
) -> Result<GateDecision> {
    let decision = security.gate_decision(class);
    tracing::debug!(
        target: "flows",
        node,
        ?class,
        ?decision,
        tier = ?security.autonomy,
        "[flows] node tier gate: evaluating autonomy-tier decision"
    );
    if decision == GateDecision::Block {
        tracing::warn!(
            target: "flows",
            node,
            ?class,
            tier = ?security.autonomy,
            "[flows] node tier gate: BLOCKED by autonomy tier — refusing before dispatch"
        );
        return Err(EngineError::Capability(format!(
            "{POLICY_BLOCKED_MARKER} flows {node} node is not permitted under the current \
             autonomy tier ({:?}): {class:?}-class actions are blocked. Raise the [autonomy] \
             tier to run this node.",
            security.autonomy
        )));
    }
    Ok(decision)
}

/// Dispatches to the process-global [`ApprovalGate`](crate::openhuman::security::approval::ApprovalGate),
/// escalating a `Prompt`-tier decision into a forced human-in-the-loop round
/// trip regardless of the running flow's own `require_approval` toggle.
///
/// **Why this is needed (Codex P1 finding):** `ApprovalGate::intercept_audited`
/// branches on the scoped [`AgentTurnOrigin`](crate::openhuman::agent::turn_origin::AgentTurnOrigin) —
/// for a `TrustedAutomation { source: Workflow { require_approval: false }, .. }`
/// origin (the default for every saved flow unless the author opts in) it
/// returns `Allow` unconditionally, the same pre-declared-trust-root shortcut a
/// user-authorized cron job gets. That shortcut is correct when the node's
/// autonomy-tier decision was itself `Allow`, but it silently defeats a
/// Supervised-tier `Prompt` decision: without this escalation, a Supervised
/// user's `http_request`/`code` node would run unattended purely because the
/// flow's `require_approval` defaults to `false` — the tier's "ask me" was
/// never actually enforced.
///
/// When `tier_decision` is [`GateDecision::Prompt`] and the current origin is a
/// `Workflow { require_approval: false }` trust root, this scopes a *for this
/// call only* `Workflow { require_approval: true }` origin around
/// `intercept_audited`, forcing the real parking/HITL flow. `GateDecision::Allow`
/// (and any other origin shape) passes through unchanged — existing behavior.
pub(crate) async fn gate_call_for_tier(
    tier_decision: GateDecision,
    tool_name: &str,
    action_summary: &str,
    args_redacted: Value,
) -> (
    crate::openhuman::security::approval::GateOutcome,
    Option<String>,
) {
    use crate::openhuman::agent::turn_origin;

    let Some(gate) = crate::openhuman::security::approval::ApprovalGate::try_global() else {
        return (
            crate::openhuman::security::approval::GateOutcome::Allow,
            None,
        );
    };

    match escalated_origin_for_prompt(tier_decision, turn_origin::current()) {
        Some(escalated) => {
            tracing::debug!(
                target: "flows",
                tool_name,
                "[flows] node tier gate: tier decision is Prompt — escalating this dispatch to a \
                 forced approval round-trip regardless of the flow's require_approval toggle"
            );
            turn_origin::with_origin(
                escalated,
                gate.intercept_audited(tool_name, action_summary, args_redacted),
            )
            .await
        }
        None => {
            gate.intercept_audited(tool_name, action_summary, args_redacted)
                .await
        }
    }
}

/// Pure decision core of [`gate_call_for_tier`]: when `tier_decision` is
/// [`GateDecision::Prompt`] and `origin` is a `Workflow { require_approval:
/// false }` trust root, returns a clone of that origin with `require_approval`
/// flipped to `true` (the forced escalation). Otherwise returns `None` — the
/// caller then dispatches through the unmodified origin, matching prior
/// behavior. Split out as a free function over plain values (no gate, no
/// task-local read) so the escalation policy is unit-testable without a live
/// `ApprovalGate`.
pub(crate) fn escalated_origin_for_prompt(
    tier_decision: GateDecision,
    origin: Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin>,
) -> Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin> {
    (tier_decision == GateDecision::Prompt)
        .then(|| force_workflow_approval(origin))
        .flatten()
}

fn force_workflow_approval(
    origin: Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin>,
) -> Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin> {
    use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

    match origin {
        Some(AgentTurnOrigin::TrustedAutomation {
            job_id,
            source:
                TrustedAutomationSource::Workflow {
                    require_approval: false,
                },
        }) => Some(AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::Workflow {
                require_approval: true,
            },
        }),
        _ => None,
    }
}

/// Pure decision core of the nested agent-node harness escalation (issue
/// #4595): when the flow run's origin is a `Workflow { require_approval: false }`
/// trust root, returns a clone with `require_approval` flipped to `true` so the
/// [`ApprovalGate`](crate::openhuman::security::approval::ApprovalGate)'s pre-declared-
/// action shortcut (`gate.rs::intercept_audited`, `Workflow { require_approval:
/// false }` → `Allow` without prompt) does NOT apply to tool calls the nested
/// harness picks at runtime.
///
/// **Why this is different from [`escalated_origin_for_prompt`].** That helper
/// escalates a *single* flow-node acting tool dispatch when the tier decision
/// is `Prompt`. This helper escalates the *entire nested harness turn*
/// unconditionally, because the flow author never pre-declared which tools the
/// referenced agent's LLM will pick — the graph only names the `agent_ref`, and
/// the definition's `ToolScope` is the runtime pool. So the "trust root =
/// static action" invariant that justifies the `intercept_audited` shortcut
/// simply doesn't hold across the `Agent::run_single` boundary.
///
/// `Workflow { require_approval: true }` passes through unchanged (already
/// user-forced HITL); other origins pass through unchanged (Cron / Web chat
/// / etc. don't route through this call site today, but if they ever do the
/// shortcut is safe or already covered by that origin's own gate branch).
/// Split out as a free function over plain values so the escalation policy is
/// unit-testable without a live `ApprovalGate`.
pub(crate) fn escalated_origin_for_nested_harness(
    origin: Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin>,
) -> Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin> {
    force_workflow_approval(origin)
}
