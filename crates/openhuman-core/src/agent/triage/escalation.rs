//! Translate a parsed classifier decision into side effects.
//!
//! The four actions:
//!
//! - **`drop`** — log only, publish `TriggerEvaluated`.
//! - **`acknowledge`** — log + publish `TriggerEvaluated`. (Memory-write
//!   for ack is a future addition.)
//! - **`react`** — dispatch the `trigger_reactor` sub-agent via
//!   [`run_subagent`], publish `TriggerEvaluated` + `TriggerEscalated`.
//! - **`escalate`** — dispatch the `orchestrator` sub-agent, same
//!   events.
//!
//! `react`/`escalate` build a full [`Agent`] from config so they have
//! a real provider, tool registry, and memory backing — the same
//! construction path `agent_chat` uses. A [`ParentExecutionContext`] is
//! carried explicitly into [`run_subagent`] so it can inherit the provider
//! and tools.

use crate::core::runtime::DispatchCtx;
use anyhow::{anyhow, Context};

use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent::harness::host_definition::AgentDefinitionRegistry;
use crate::agent::orchestration::parent_context::build_root_parent;
use crate::agent::subagent_host::{self, SubagentRunOptions};
use crate::config::Config;

use super::decision::TriageAction;
use super::envelope::{TriggerEnvelope, TriggerSource};
use super::evaluator::TriageRun;
use super::events;

/// Executes the side effects of a triage decision.
///
/// This function is responsible for:
/// 1. Publishing the `TriggerEvaluated` telemetry event.
/// 2. Logging the classification outcome.
/// 3. If the action is `React` or `Escalate`, dispatching the appropriate
///    sub-agent (`trigger_reactor` or `orchestrator`).
/// 4. Publishing `TriggerEscalated` or `TriggerEscalationFailed` events.
pub async fn apply_decision(
    ctx: &DispatchCtx,
    run: TriageRun,
    envelope: &TriggerEnvelope,
) -> anyhow::Result<()> {
    // Always publish `TriggerEvaluated` — it's the single source of
    // truth for dashboards, counts every trigger regardless of action.
    events::publish_evaluated(
        envelope,
        run.decision.action.as_str(),
        run.used_local,
        run.latency_ms,
    );

    match run.decision.action {
        TriageAction::Drop => {
            tracing::debug!(
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                reason = %run.decision.reason,
                "[triage::escalation] DROP — no downstream work"
            );
        }
        TriageAction::Acknowledge => {
            // Acknowledge is a classification, not a write. What the trigger
            // *was* is already durable in the composio trigger-history JSONL,
            // and what it was *judged to be* went out as `TriggerEvaluated`
            // above. Copying a summary into the memory store on top of that
            // would duplicate a document the connector sync already ingested —
            // same mail, second copy, competing for the same recall slots — so
            // this arm deliberately writes nothing.
            //
            // What survives differs by source, so the log says which — only the
            // composio path archives its input, and claiming otherwise sends an
            // operator looking for a record that was never written.
            //
            // What is genuinely missing is not a copy of the input but a record
            // of what happened *after* the verdict, for every action and every
            // source, not just this one. That belongs in a progress surface of
            // its own rather than bolted onto the acknowledge branch (#5408).
            tracing::info!(
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                retained = %retained_input_note(&envelope.source),
                reason = %run.decision.reason,
                "[triage::escalation] ACKNOWLEDGE — no autonomous action; \
                 recorded as TriggerEvaluated"
            );
        }
        TriageAction::React | TriageAction::Escalate => {
            let target = run
                .decision
                .target_agent
                .as_deref()
                .unwrap_or("trigger_reactor");
            let prompt = run.decision.prompt.as_deref().unwrap_or("");
            let action_str = run.decision.action.as_str().to_uppercase();

            tracing::info!(
                action = %action_str,
                target_agent = %target,
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                prompt_chars = prompt.chars().count(),
                reason = %run.decision.reason,
                "[triage::escalation] dispatching sub-agent"
            );

            // ── External-effect approval gate (#1339) ─────────
            // React / Escalate fire a sub-agent that may call
            // external-effect tools on the user's behalf. Catching
            // here as well as at tool-loop level lets the user
            // decline the whole escalation up-front instead of one
            // tool call at a time. The per-tool gate further down
            // still applies — defense in depth, not duplication
            // (each gate is short-circuited by the session
            // allowlist after the first approval).
            let mut approval_request_id: Option<String> = None;
            let mut approval_gate_for_audit: Option<
                std::sync::Arc<crate::security::approval::ApprovalGate>,
            > = None;
            if let Some(gate) = crate::security::approval::ApprovalGate::try_global() {
                let summary = format!(
                    "triage::{} target={} prompt_chars={}",
                    action_str,
                    target,
                    prompt.chars().count()
                );
                let redacted = serde_json::json!({
                    "action": action_str,
                    "target_agent": target,
                    "external_id": envelope.external_id,
                    "label": envelope.display_label,
                    "prompt_chars": prompt.chars().count(),
                });
                let tool_key = format!("triage.{}", run.decision.action.as_str());
                // A triage escalation is not a chat turn: nothing to route an
                // approval reply to.
                let (outcome, request_id) = gate
                    .intercept_audited(&Default::default(), &tool_key, &summary, redacted)
                    .await;
                match outcome {
                    crate::security::approval::GateOutcome::Allow => {
                        approval_request_id = request_id;
                        if approval_request_id.is_some() {
                            approval_gate_for_audit = Some(gate);
                        }
                    }
                    crate::security::approval::GateOutcome::Deny { reason } => {
                        tracing::warn!(
                            action = %action_str,
                            target_agent = %target,
                            external_id = %envelope.external_id,
                            reason = %reason,
                            "[triage::escalation] approval gate denied dispatch"
                        );
                        events::publish_failed(
                            envelope,
                            &format!("approval denied for `{target}`: {reason}"),
                        );
                        return Ok(());
                    }
                }
            }

            let dispatch_result = dispatch_target_agent(ctx, target, prompt).await;
            // Record terminal status on the approval audit row
            // (#2135). Best-effort: write errors are logged inside
            // record_execution and never propagate to the caller.
            if let (Some(gate), Some(req_id)) = (
                approval_gate_for_audit.as_ref(),
                approval_request_id.as_ref(),
            ) {
                let (exec_outcome, err_text) = match &dispatch_result {
                    Ok(_) => (crate::security::approval::ExecutionOutcome::Success, None),
                    Err(e) => (
                        crate::security::approval::ExecutionOutcome::Failure,
                        Some(e.to_string()),
                    ),
                };
                gate.record_execution(req_id, exec_outcome, err_text.as_deref());
            }
            match dispatch_result {
                Ok(output) => {
                    tracing::info!(
                        target_agent = %target,
                        output_chars = output.chars().count(),
                        "[triage::escalation] sub-agent completed"
                    );
                    events::publish_escalated(envelope, target);
                }
                Err(err) => {
                    tracing::error!(
                        target_agent = %target,
                        error = %err,
                        "[triage::escalation] sub-agent dispatch failed"
                    );
                    events::publish_failed(
                        envelope,
                        &format!("sub-agent `{target}` failed: {err}"),
                    );
                    return Err(err);
                }
            }
        }
    }
    Ok(())
}

/// Build a full [`Agent`] from config, install a [`ParentExecutionContext`]
/// on the task-local, and call [`run_subagent`] with the named definition
/// and prompt.
///
/// This is heavier than a simple `agent.run_turn` bus call — it creates a
/// provider, memory store, tool registry, and all the machinery `Agent`
/// normally needs. The cost is acceptable because `react`/`escalate`
/// triggers are relatively rare (most triggers are `drop`/`acknowledge`)
/// and the construction is the same O(1) code path `agent_chat` uses.
/// Build the triage root parent: the shared [`build_root_parent`] context with
/// nested spawns scoped to the single dispatched target agent.
///
/// This collapses the previously hand-rolled ~20-field [`ParentExecutionContext`]
/// literal onto the shared builder so the two can't drift (#4369). The identity
/// fields come straight from `build_root_parent` and match the old literal
/// exactly: `agent_definition_id`/`channel` = `"triage"`, `session_id` =
/// `"triage-{uuid}"`, and the session-key chain + PFormat tool-call format are
/// inherited from the config-built agent. The **only** field triage overrides is
/// `allowed_subagent_ids`: the escalated agent may itself nested-spawn only the
/// dispatched target (the builder defaults this to empty for background roots).
async fn build_triage_parent(
    ctx: &DispatchCtx,
    config: &Config,
    agent_id: &str,
) -> anyhow::Result<ParentExecutionContext> {
    let mut parent_ctx = build_root_parent(ctx, config, "triage", "triage", "triage")
        .await
        .context("building root parent for sub-agent dispatch")?;
    parent_ctx.allowed_subagent_ids = [agent_id.to_string()].into_iter().collect();
    Ok(parent_ctx)
}

async fn dispatch_target_agent(
    ctx: &DispatchCtx,
    agent_id: &str,
    prompt: &str,
) -> anyhow::Result<String> {
    #[cfg(test)]
    if agent_id.starts_with("missing-agent-") {
        return Err(anyhow!(
            "agent definition `{agent_id}` not found in registry"
        ));
    }

    let config = crate::config::ops::load_current_or_init(ctx)
        .await
        .context("loading config for sub-agent dispatch")?;

    let parent_ctx = build_triage_parent(ctx, &config, agent_id).await?;

    // `build_root_parent` (inside `build_triage_parent`) guarantees the registry
    // is initialised, so this lookup for the target definition is safe here.
    let registry = AgentDefinitionRegistry::current()
        .ok_or_else(|| anyhow!("AgentDefinitionRegistry not initialised"))?;
    let definition = registry
        .get(agent_id)
        .ok_or_else(|| anyhow!("agent definition `{agent_id}` not found in registry"))?;

    tracing::debug!(
        agent_id = %agent_id,
        model = %parent_ctx.model_name,
        tool_count = parent_ctx.all_tools.len(),
        "[triage::escalation] dispatching run_subagent with parent context"
    );

    let outcome = subagent_host::run_subagent(
        definition,
        prompt,
        SubagentRunOptions {
            run_context: crate::agent::tinyagents::host::OpenHumanRunContext::new()
                .with_core(ctx.core().cloned())
                .with_host_env(&parent_ctx.host_env)
                .with_parent(parent_ctx),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| anyhow!("run_subagent(`{agent_id}`) failed: {e}"))?;

    tracing::debug!(
        agent_id = %agent_id,
        elapsed_ms = outcome.elapsed.as_millis() as u64,
        iterations = outcome.iterations,
        output_chars = outcome.output.chars().count(),
        "[triage::escalation] run_subagent completed"
    );

    Ok(outcome.output)
}

/// What survives an acknowledged trigger, for the source it actually came from.
///
/// The composio webhook path archives every event to a daily JSONL before the
/// triage gates, so its input is durable whatever the verdict. No other source
/// has an equivalent: a webhook, cron, webview, or external trigger leaves only
/// the `TriggerEvaluated` event. Saying "retained in trigger history" for all
/// of them would tell an operator to go looking for a record that was never
/// written.
fn retained_input_note(source: &TriggerSource) -> &'static str {
    match source {
        TriggerSource::Composio { .. } => "trigger-history archive",
        _ => "none — verdict only",
    }
}

#[cfg(test)]
#[path = "escalation_tests.rs"]
mod tests;
