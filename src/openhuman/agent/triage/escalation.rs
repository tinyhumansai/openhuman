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
//! installed on the task-local so [`run_subagent`] can inherit the
//! provider and tools.

use std::sync::Arc;

use anyhow::{anyhow, Context};

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::agent::harness::subagent_runner::{self, SubagentRunOptions};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;

use super::decision::TriageAction;
use super::envelope::{TaskCardLink, TriggerEnvelope};
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
pub async fn apply_decision(run: TriageRun, envelope: &TriggerEnvelope) -> anyhow::Result<()> {
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
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                reason = %run.decision.reason,
                "[triage::escalation] DROP — no downstream work"
            );
            // A dropped trigger that carries a board card (proactive
            // task-source ingest) must be terminally gated, or the board
            // poller — which dispatches any `Todo`/`Ready` card regardless of
            // the triage verdict — would re-run it on the next tick, silently
            // breaking the noise-gating contract documented on
            // `SourceTarget::AgentTodoProactive`.
            gate_linked_card_terminal(envelope, "drop");
        }
        TriageAction::Acknowledge => {
            tracing::info!(
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                reason = %run.decision.reason,
                "[triage::escalation] ACKNOWLEDGE — logged (memory-write is a future addition)"
            );
            // Acknowledge means "seen, no autonomous action needed" — same as
            // drop, the linked card must not be picked up by the board poller.
            gate_linked_card_terminal(envelope, "acknowledge");
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

            // ── Unified task-board path ───────────────────────
            // A trigger linked to a board card is handed to the deterministic
            // dispatcher (claim → autonomous run → write-back). The claim
            // (todo→in_progress) deduplicates against the board poller, so
            // firing both is safe. Non-card triggers (composio/webhook/cron)
            // fall through to the one-shot triage sub-agent below.
            if let Some(link) = &envelope.card_link {
                use crate::openhuman::agent::task_dispatcher::DispatchOutcome;
                match dispatch_linked_card(link).await {
                    Ok(DispatchOutcome::Running { run_id }) => {
                        tracing::info!(
                            card_id = %link.card_id,
                            run_id = %run_id,
                            "[triage::escalation] task-card dispatched to deterministic runner"
                        );
                        events::publish_escalated(envelope, "task_dispatcher");
                    }
                    Ok(DispatchOutcome::AwaitingApproval) => {
                        // Parked for plan approval (autonomy gate). Not an
                        // escalation yet — the approval flow resumes it.
                        tracing::info!(
                            card_id = %link.card_id,
                            "[triage::escalation] task-card parked awaiting plan approval"
                        );
                    }
                    Err(reason) => {
                        // A failed claim (another card already in progress, or
                        // the card vanished) is benign — the poller retries.
                        tracing::info!(
                            card_id = %link.card_id,
                            reason = %reason,
                            "[triage::escalation] task-card dispatch skipped (claim failed?)"
                        );
                    }
                }
                return Ok(());
            }

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
                std::sync::Arc<crate::openhuman::approval::ApprovalGate>,
            > = None;
            if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
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
                let (outcome, request_id) =
                    gate.intercept_audited(&tool_key, &summary, redacted).await;
                match outcome {
                    crate::openhuman::approval::GateOutcome::Allow => {
                        approval_request_id = request_id;
                        if approval_request_id.is_some() {
                            approval_gate_for_audit = Some(gate);
                        }
                    }
                    crate::openhuman::approval::GateOutcome::Deny { reason } => {
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

            let dispatch_result = dispatch_target_agent(target, prompt).await;
            // Record terminal status on the approval audit row
            // (#2135). Best-effort: write errors are logged inside
            // record_execution and never propagate to the caller.
            if let (Some(gate), Some(req_id)) = (
                approval_gate_for_audit.as_ref(),
                approval_request_id.as_ref(),
            ) {
                let (exec_outcome, err_text) = match &dispatch_result {
                    Ok(_) => (crate::openhuman::approval::ExecutionOutcome::Success, None),
                    Err(e) => (
                        crate::openhuman::approval::ExecutionOutcome::Failure,
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
async fn dispatch_target_agent(agent_id: &str, prompt: &str) -> anyhow::Result<String> {
    let config = Config::load_or_init()
        .await
        .context("loading config for sub-agent dispatch")?;

    let mut agent =
        Agent::from_config(&config).context("building Agent from config for sub-agent dispatch")?;

    // Populate connected integrations from the process-wide cache (or a
    // fresh fetch if cold) so triage-triggered sub-agents see the real
    // integrations in their system prompts.
    let integrations = crate::openhuman::composio::fetch_connected_integrations(&config).await;
    agent.set_connected_integrations(integrations);

    let registry = AgentDefinitionRegistry::global()
        .ok_or_else(|| anyhow!("AgentDefinitionRegistry not initialised"))?;
    let definition = registry
        .get(agent_id)
        .ok_or_else(|| anyhow!("agent definition `{agent_id}` not found in registry"))?;

    // Build the ParentExecutionContext from the Agent's public accessors
    // so `run_subagent` can inherit the provider, tools, memory, etc.
    let parent_ctx = ParentExecutionContext {
        provider: agent.provider_arc(),
        all_tools: agent.tools_arc(),
        all_tool_specs: agent.tool_specs_arc(),
        model_name: agent.model_name().to_string(),
        temperature: agent.temperature(),
        workspace_dir: agent.workspace_dir().to_path_buf(),
        memory: agent.memory_arc(),
        agent_config: agent.agent_config().clone(),
        skills: Arc::new(agent.skills().to_vec()),
        memory_context: Arc::new(None), // Sub-agent queries memory via tools if needed
        session_id: format!("triage-{}", uuid::Uuid::new_v4()),
        channel: "triage".to_string(),
        connected_integrations: agent.connected_integrations().to_vec(),
        // Triage runs sub-agents with the parent's existing dispatcher
        // — fall back to PFormat if no accessor is available. Triage
        // doesn't currently spawn anything that depends on the new
        // dispatcher-aware sub-agent renderer.
        tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
        // Triage inherits the parent's session-key chain so escalated
        // sub-agents write their transcripts alongside the parent's,
        // preserving the `{parent}__{child}.jsonl` hierarchy.
        session_key: agent.session_key().to_string(),
        session_parent_prefix: agent.session_parent_prefix().map(str::to_string),
        // Triage runs sub-agents synchronously without streaming progress
        // back to a UI; the runner skips child-progress emission when this
        // is `None`.
        on_progress: None,
    };

    tracing::debug!(
        agent_id = %agent_id,
        model = %parent_ctx.model_name,
        tool_count = parent_ctx.all_tools.len(),
        "[triage::escalation] dispatching run_subagent with parent context"
    );

    let outcome = with_parent_context(parent_ctx, async {
        subagent_runner::run_subagent(definition, prompt, SubagentRunOptions::default()).await
    })
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

/// Load the linked card from its board and hand it to the deterministic task
/// dispatcher (claim → autonomous run → write-back). Errors (card not found,
/// or claim rejected because another card is already in progress) propagate to
/// the caller, which treats them as benign skips.
async fn dispatch_linked_card(
    link: &TaskCardLink,
) -> Result<crate::openhuman::agent::task_dispatcher::DispatchOutcome, String> {
    let snapshot = crate::openhuman::todos::ops::list(&link.location)?;
    let card = snapshot
        .cards
        .into_iter()
        .find(|c| c.id == link.card_id)
        .ok_or_else(|| format!("card `{}` not found on board", link.card_id))?;
    crate::openhuman::agent::task_dispatcher::dispatch_card(link.location.clone(), card).await
}

/// Terminally gate a card-linked trigger that triage decided to `drop` /
/// `acknowledge`, so the board poller (which dispatches any pending
/// `Todo`/`Ready` card) won't re-run it. Only a still-pending card is gated;
/// if it already advanced (the poller claimed it, or it's already terminal)
/// it is left untouched. Best-effort: a missing card or write failure is
/// logged, never propagated — the trigger was already evaluated.
fn gate_linked_card_terminal(envelope: &TriggerEnvelope, decision: &str) {
    use crate::openhuman::agent::task_board::TaskCardStatus;
    use crate::openhuman::todos::ops;

    let Some(link) = &envelope.card_link else {
        return;
    };

    let current = match ops::list(&link.location) {
        Ok(snapshot) => snapshot
            .cards
            .into_iter()
            .find(|c| c.id == link.card_id)
            .map(|c| c.status),
        Err(e) => {
            tracing::warn!(
                card_id = %link.card_id,
                error = %e,
                "[triage::escalation] reload before gating linked card failed"
            );
            return;
        }
    };

    match current {
        Some(TaskCardStatus::Todo | TaskCardStatus::Ready | TaskCardStatus::AwaitingApproval) => {
            match ops::update_status(&link.location, &link.card_id, TaskCardStatus::Rejected) {
                Ok(_) => tracing::info!(
                    card_id = %link.card_id,
                    decision = %decision,
                    "[triage::escalation] gated task-card → rejected (poller will skip)"
                ),
                Err(e) => tracing::warn!(
                    card_id = %link.card_id,
                    decision = %decision,
                    error = %e,
                    "[triage::escalation] failed to gate task-card (poller may re-dispatch)"
                ),
            }
        }
        other => tracing::debug!(
            card_id = %link.card_id,
            decision = %decision,
            status = ?other,
            "[triage::escalation] linked task-card not pending; no gating needed"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::{init_global, DomainEvent};
    use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
    use serde_json::json;
    use tokio::time::{sleep, timeout, Duration};

    static TEST_EVENTS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct TestEventsGuard(tokio::sync::MutexGuard<'static, ()>);

    impl Drop for TestEventsGuard {
        fn drop(&mut self) {
            events::clear_test_events();
        }
    }

    async fn test_events_guard() -> TestEventsGuard {
        let guard = TEST_EVENTS_LOCK.lock().await;
        events::clear_test_events();
        TestEventsGuard(guard)
    }

    fn envelope(external_id: &str) -> TriggerEnvelope {
        TriggerEnvelope::from_composio(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "triage-escalation",
            external_id,
            json!({ "subject": "hello" }),
        )
    }

    fn run(action: TriageAction) -> TriageRun {
        TriageRun {
            decision: super::super::decision::TriageDecision {
                action,
                target_agent: None,
                prompt: None,
                reason: "because".into(),
            },
            used_local: false,
            latency_ms: 9,
            resolution_path: super::super::evaluator::TriageResolutionPath::Cloud,
        }
    }

    fn run_with_target(action: TriageAction, target_agent: &str, prompt: &str) -> TriageRun {
        TriageRun {
            decision: super::super::decision::TriageDecision {
                action,
                target_agent: Some(target_agent.into()),
                prompt: Some(prompt.into()),
                reason: "because".into(),
            },
            used_local: false,
            latency_ms: 9,
            resolution_path: super::super::evaluator::TriageResolutionPath::Cloud,
        }
    }

    async fn collect_trigger_events_until(
        external_id: &str,
        expected: impl Fn(&[DomainEvent]) -> bool,
    ) -> Vec<DomainEvent> {
        let external_id = external_id.to_string();
        timeout(Duration::from_secs(5), async {
            loop {
                let captured = events::test_events_for_external_id(&external_id);
                if expected(&captured) {
                    return captured;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("expected triage event should arrive")
    }

    #[tokio::test]
    async fn apply_decision_drop_only_publishes_evaluated() {
        let _events_guard = test_events_guard().await;
        let envelope = envelope("esc-drop");
        let _ = init_global(32);
        let collect = tokio::spawn(collect_trigger_events_until("esc-drop", |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::TriggerEvaluated {
                        decision,
                        external_id,
                        ..
                    } if decision == "drop" && external_id == "esc-drop"
                )
            })
        }));

        apply_decision(run(TriageAction::Drop), &envelope)
            .await
            .expect("drop should not fail");

        let captured = collect.await.expect("event collector should not panic");
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEvaluated {
                decision,
                external_id,
                ..
            } if decision == "drop" && external_id == "esc-drop"
        )));
        assert!(!captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEscalated { external_id, .. }
                | DomainEvent::TriggerEscalationFailed { external_id, .. }
                if external_id == "esc-drop"
        )));
    }

    #[tokio::test]
    async fn apply_decision_acknowledge_only_publishes_evaluated() {
        let _events_guard = test_events_guard().await;
        let envelope = envelope("esc-ack");
        let _ = init_global(32);
        let collect = tokio::spawn(collect_trigger_events_until("esc-ack", |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::TriggerEvaluated {
                        decision,
                        external_id,
                        ..
                    } if decision == "acknowledge" && external_id == "esc-ack"
                )
            })
        }));

        apply_decision(run(TriageAction::Acknowledge), &envelope)
            .await
            .expect("acknowledge should not fail");

        let captured = collect.await.expect("event collector should not panic");
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEvaluated {
                decision,
                external_id,
                ..
            } if decision == "acknowledge" && external_id == "esc-ack"
        )));
        assert!(!captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEscalated { external_id, .. }
                | DomainEvent::TriggerEscalationFailed { external_id, .. }
                if external_id == "esc-ack"
        )));
    }

    fn seed_task_card() -> (
        tempfile::TempDir,
        crate::openhuman::todos::ops::BoardLocation,
        String,
    ) {
        use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch};
        let dir = tempfile::tempdir().unwrap();
        let location = BoardLocation::Thread {
            workspace_dir: dir.path().to_path_buf(),
            thread_id: "task-sources".to_string(),
        };
        let card_id = ops::add(&location, "ingested issue", CardPatch::default())
            .unwrap()
            .cards[0]
            .id
            .clone();
        (dir, location, card_id)
    }

    #[tokio::test]
    async fn apply_decision_drop_gates_linked_card_to_rejected() {
        use crate::openhuman::agent::task_board::TaskCardStatus;
        use crate::openhuman::todos::ops;

        let _events_guard = test_events_guard().await;
        let _ = init_global(32);
        let (_dir, location, card_id) = seed_task_card();

        let envelope = envelope("esc-drop-card").with_task_card(card_id.clone(), location.clone());
        apply_decision(run(TriageAction::Drop), &envelope)
            .await
            .expect("drop should not fail");

        let status = ops::list(&location)
            .unwrap()
            .cards
            .into_iter()
            .find(|c| c.id == card_id)
            .map(|c| c.status);
        assert_eq!(
            status,
            Some(TaskCardStatus::Rejected),
            "a dropped card-linked trigger must be gated terminally so the board poller skips it"
        );
    }

    #[tokio::test]
    async fn apply_decision_acknowledge_gates_linked_card_to_rejected() {
        use crate::openhuman::agent::task_board::TaskCardStatus;
        use crate::openhuman::todos::ops;

        let _events_guard = test_events_guard().await;
        let _ = init_global(32);
        let (_dir, location, card_id) = seed_task_card();

        let envelope = envelope("esc-ack-card").with_task_card(card_id.clone(), location.clone());
        apply_decision(run(TriageAction::Acknowledge), &envelope)
            .await
            .expect("acknowledge should not fail");

        let status = ops::list(&location)
            .unwrap()
            .cards
            .into_iter()
            .find(|c| c.id == card_id)
            .map(|c| c.status);
        assert_eq!(status, Some(TaskCardStatus::Rejected));
    }

    #[tokio::test]
    async fn apply_decision_react_failure_publishes_failed_event() {
        let _events_guard = test_events_guard().await;
        let envelope = envelope("esc-react-fail");
        let _ = init_global(32);
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let collect = tokio::spawn(collect_trigger_events_until("esc-react-fail", |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::TriggerEvaluated {
                        decision,
                        external_id,
                        ..
                    } if decision == "react" && external_id == "esc-react-fail"
                )
            }) && events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::TriggerEscalationFailed { external_id, reason, .. }
                        if external_id == "esc-react-fail" && reason.contains("missing-agent")
                )
            })
        }));

        let err = apply_decision(
            run_with_target(TriageAction::React, "missing-agent", "handle this"),
            &envelope,
        )
        .await
        .expect_err("missing target agent should fail");
        assert!(err.to_string().contains("missing-agent"));

        let captured = collect.await.expect("event collector should not panic");
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEvaluated {
                decision,
                external_id,
                ..
            } if decision == "react" && external_id == "esc-react-fail"
        )));
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEscalationFailed { external_id, reason, .. }
                if external_id == "esc-react-fail" && reason.contains("missing-agent")
        )));
    }

    #[tokio::test]
    async fn apply_decision_escalate_failure_publishes_failed_event() {
        let _events_guard = test_events_guard().await;
        let envelope = envelope("esc-escalate-fail");
        let _ = init_global(32);
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let collect = tokio::spawn(collect_trigger_events_until(
            "esc-escalate-fail",
            |events| {
                events.iter().any(|event| {
                    matches!(
                event,
                DomainEvent::TriggerEvaluated {
                    decision,
                    external_id,
                    ..
                } if decision == "escalate" && external_id == "esc-escalate-fail"
            )
                }) && events.iter().any(|event| matches!(
                event,
                DomainEvent::TriggerEscalationFailed { external_id, reason, .. }
                    if external_id == "esc-escalate-fail" && reason.contains("missing-agent")
            ))
            },
        ));

        let err = apply_decision(
            run_with_target(TriageAction::Escalate, "missing-agent", "escalate this"),
            &envelope,
        )
        .await
        .expect_err("missing orchestrator target should fail");
        assert!(err.to_string().contains("missing-agent"));

        let captured = collect.await.expect("event collector should not panic");
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEvaluated {
                decision,
                external_id,
                ..
            } if decision == "escalate" && external_id == "esc-escalate-fail"
        )));
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEscalationFailed { external_id, reason, .. }
                if external_id == "esc-escalate-fail" && reason.contains("missing-agent")
        )));
    }
}
