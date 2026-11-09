//! Autonomy-tier and approval gating for flow nodes.
//!
//! Every node that reaches outside the process — a tool call, an HTTP request,
//! a code run — passes through here first. The tier decides whether the action
//! is allowed outright, needs a human, or is refused; the approval gate is what
//! actually performs the round-trip when a human is needed.

use super::{EngineError, Result};
use serde_json::Value;

use crate::security::policy::CommandClass;
use crate::security::{GateDecision, SecurityPolicy};
use tinytools::status::POLICY_BLOCKED_MARKER;

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
/// [`OpenHumanMemory`](super::OpenHumanMemory) adapter
/// reuses this exact function — `CommandClass::Read` for
/// recall/search/flavour/people, `CommandClass::Write` for remember/forget —
/// rather than growing a second permission path for the new node kind.
pub(crate) async fn enforce_node_tier_gate(
    security: &SecurityPolicy,
    class: CommandClass,
    node: &str,
) -> Result<GateDecision> {
    let decision = security.command_class_gate(class).await.map_err(|error| {
        EngineError::Capability(format!(
            "{POLICY_BLOCKED_MARKER} TinySecurity could not authorize the {node} node: {error}"
        ))
    })?;
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

/// Loads the tier visible to one flow dispatch while keeping the policy's
/// workspace roots, action budget tracker, and other fixed fields from the
/// capability's original policy.
pub(crate) async fn current_tier_policy(
    ctx: &crate::core::runtime::DispatchCtx,
    captured_config: &crate::config::Config,
    baseline: &SecurityPolicy,
) -> Result<SecurityPolicy> {
    let live = match ctx.autonomy_tier_source() {
        crate::core::runtime::AutonomyTierSource::Captured => None,
        crate::core::runtime::AutonomyTierSource::LiveConfig => {
            let live = crate::config::ops::reload_config_snapshot_with_timeout(captured_config)
                .await
                .map_err(|error| tier_config_error(&error))?;
            if live.recovered_from_corruption {
                return Err(tier_config_error(
                    "live config was recovered from corruption",
                ));
            }
            Some(live)
        }
    };
    tier_policy_from_config(ctx, captured_config, live.as_ref(), baseline)
}

/// Uses an already reloaded live config when the caller needs that same
/// snapshot for another part of its operation (Composio integrations).
pub(crate) fn tier_policy_from_config(
    ctx: &crate::core::runtime::DispatchCtx,
    _captured_config: &crate::config::Config,
    live_config: Option<&crate::config::Config>,
    baseline: &SecurityPolicy,
) -> Result<SecurityPolicy> {
    let mut policy = baseline.clone();
    let ceiling = ctx.autonomy_tier_ceiling();
    if ctx.autonomy_tier_source() == crate::core::runtime::AutonomyTierSource::Captured {
        if let Some(ceiling) = ceiling {
            if policy.enabled {
                policy.autonomy = stricter_tier(policy.autonomy, ceiling);
            } else if ceiling != crate::security::AutonomyLevel::Full {
                policy.enabled = true;
                policy.autonomy = ceiling;
            }
        }
        return Ok(policy);
    }

    let config =
        live_config.ok_or_else(|| tier_config_error("live config snapshot was not provided"))?;
    if config.autonomy.enabled {
        policy.enabled = true;
        policy.autonomy = config.autonomy.level;
        if let Some(ceiling) = ceiling {
            policy.autonomy = stricter_tier(policy.autonomy, ceiling);
        }
    } else if let Some(
        ceiling @ (crate::security::AutonomyLevel::ReadOnly
        | crate::security::AutonomyLevel::Supervised),
    ) = ceiling
    {
        // A disabled config contributes no tier restriction; a restrictive
        // host ceiling remains independently active.
        policy.enabled = true;
        policy.autonomy = ceiling;
    } else {
        // A Full ceiling adds no restriction, so it cannot enable a tier which
        // the live config explicitly disabled.
        policy.enabled = false;
        policy.autonomy = crate::security::AutonomyLevel::Full;
    }
    Ok(policy)
}

/// Async counterpart used by the host adapters. Config failures are policy
/// failures and carry the harness's permanent-block marker so the run cannot
/// proceed using a stale permissive tier.
pub(crate) async fn enforce_current_node_tier_gate(
    ctx: &crate::core::runtime::DispatchCtx,
    captured_config: &crate::config::Config,
    baseline: &SecurityPolicy,
    class: CommandClass,
    node: &str,
) -> Result<GateDecision> {
    let policy = current_tier_policy(ctx, captured_config, baseline).await?;
    enforce_node_tier_gate(&policy, class, node).await
}

fn tier_config_error(_error: &str) -> EngineError {
    EngineError::Capability(format!(
        "{POLICY_BLOCKED_MARKER} flows autonomy tier could not be refreshed; refusing the node"
    ))
}

/// Finish the audit row opened by a gated capability after its side effect
/// settles. Gate decisions without an audit id remain a no-op.
pub(crate) fn close_audit<T, E: std::fmt::Display>(
    audit_id: Option<&str>,
    result: &std::result::Result<T, E>,
) {
    let Some(audit_id) = audit_id else {
        return;
    };
    let Some(gate) = crate::security::approval::ApprovalGate::try_global() else {
        return;
    };
    let (outcome, error) = match result {
        Ok(_) => (crate::security::approval::ExecutionOutcome::Success, None),
        Err(error) => (
            crate::security::approval::ExecutionOutcome::Failure,
            Some(error.to_string()),
        ),
    };
    gate.record_execution(audit_id, outcome, error.as_deref());
}

fn stricter_tier(
    tier: crate::security::AutonomyLevel,
    ceiling: crate::security::AutonomyLevel,
) -> crate::security::AutonomyLevel {
    use crate::security::AutonomyLevel::{Full, ReadOnly, Supervised};

    match (tier, ceiling) {
        (ReadOnly, _) | (_, ReadOnly) => ReadOnly,
        (Supervised, _) | (_, Supervised) => Supervised,
        (Full, Full) => Full,
    }
}

/// Dispatches to the process-global [`ApprovalGate`](crate::security::approval::ApprovalGate),
/// escalating a `Prompt`-tier decision into a forced human-in-the-loop round
/// trip regardless of the running flow's own `require_approval` toggle.
///
/// **Why this is needed (Codex P1 finding):** `ApprovalGate::intercept_audited`
/// branches on the scoped [`AgentTurnOrigin`](crate::core::runtime::turn_origin::AgentTurnOrigin) —
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
    ctx: &crate::core::runtime::DispatchCtx,
    approval: &crate::security::approval::ApprovalTurnContext,
    tier_decision: GateDecision,
    tool_name: &str,
    action_summary: &str,
    args_redacted: Value,
) -> (crate::security::approval::GateOutcome, Option<String>) {
    use crate::core::runtime::turn_origin;

    let Some(gate) = crate::security::approval::ApprovalGate::try_global() else {
        return (crate::security::approval::GateOutcome::Allow, None);
    };

    match escalated_origin_for_prompt(tier_decision, ctx.turn_origin()) {
        Some(escalated) => {
            tracing::debug!(
                target: "flows",
                tool_name,
                "[flows] node tier gate: tier decision is Prompt — escalating this dispatch to a \
                 forced approval round-trip regardless of the flow's require_approval toggle"
            );
            turn_origin::with_origin(
                escalated,
                gate.intercept_audited(approval, tool_name, action_summary, args_redacted),
            )
            .await
        }
        None => {
            gate.intercept_audited(approval, tool_name, action_summary, args_redacted)
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
    origin: Option<crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> Option<crate::core::runtime::turn_origin::AgentTurnOrigin> {
    (tier_decision == GateDecision::Prompt)
        .then(|| force_workflow_approval(origin))
        .flatten()
}

fn force_workflow_approval(
    origin: Option<crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> Option<crate::core::runtime::turn_origin::AgentTurnOrigin> {
    use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

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
/// [`ApprovalGate`](crate::security::approval::ApprovalGate)'s pre-declared-
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
/// simply doesn't hold across the `OpenHumanSessionHost::run_single` boundary.
///
/// `Workflow { require_approval: true }` passes through unchanged (already
/// user-forced HITL); other origins pass through unchanged (Cron / Web chat
/// / etc. don't route through this call site today, but if they ever do the
/// shortcut is safe or already covered by that origin's own gate branch).
/// Split out as a free function over plain values so the escalation policy is
/// unit-testable without a live `ApprovalGate`.
pub(crate) fn escalated_origin_for_nested_harness(
    origin: Option<crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> Option<crate::core::runtime::turn_origin::AgentTurnOrigin> {
    force_workflow_approval(origin)
}
