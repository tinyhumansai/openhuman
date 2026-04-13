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
use super::envelope::TriggerEnvelope;
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
        }
        TriageAction::Acknowledge => {
            tracing::info!(
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                reason = %run.decision.reason,
                "[triage::escalation] ACKNOWLEDGE — logged (memory-write is a future addition)"
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

            match dispatch_target_agent(target, prompt).await {
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
    let integrations =
        crate::openhuman::composio::fetch_connected_integrations(&config).await;
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
        memory_context: None, // Sub-agent queries memory via tools if needed
        session_id: format!("triage-{}", uuid::Uuid::new_v4()),
        channel: "triage".to_string(),
        connected_integrations: agent.connected_integrations().to_vec(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::{global, init_global, DomainEvent};
    use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{sleep, Duration};

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
        }
    }

    #[tokio::test]
    async fn apply_decision_drop_only_publishes_evaluated() {
        let envelope = envelope("esc-drop");
        let _ = init_global(32);
        let seen = Arc::new(Mutex::new(Vec::<DomainEvent>::new()));
        let seen_handler = Arc::clone(&seen);
        let _handle = global()
            .unwrap()
            .on("triage-escalation-drop", move |event| {
                let seen = Arc::clone(&seen_handler);
                let cloned = event.clone();
                Box::pin(async move {
                    seen.lock().await.push(cloned);
                })
            });

        apply_decision(run(TriageAction::Drop), &envelope)
            .await
            .expect("drop should not fail");
        sleep(Duration::from_millis(20)).await;

        let captured = seen.lock().await;
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
        let envelope = envelope("esc-ack");
        let _ = init_global(32);
        let seen = Arc::new(Mutex::new(Vec::<DomainEvent>::new()));
        let seen_handler = Arc::clone(&seen);
        let _handle = global().unwrap().on("triage-escalation-ack", move |event| {
            let seen = Arc::clone(&seen_handler);
            let cloned = event.clone();
            Box::pin(async move {
                seen.lock().await.push(cloned);
            })
        });

        apply_decision(run(TriageAction::Acknowledge), &envelope)
            .await
            .expect("acknowledge should not fail");
        sleep(Duration::from_millis(20)).await;

        let captured = seen.lock().await;
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

    #[tokio::test]
    async fn apply_decision_react_failure_publishes_failed_event() {
        let envelope = envelope("esc-react-fail");
        let _ = init_global(32);
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let seen = Arc::new(Mutex::new(Vec::<DomainEvent>::new()));
        let seen_handler = Arc::clone(&seen);
        let _handle = global()
            .unwrap()
            .on("triage-escalation-react-fail", move |event| {
                let seen = Arc::clone(&seen_handler);
                let cloned = event.clone();
                Box::pin(async move {
                    seen.lock().await.push(cloned);
                })
            });

        let err = apply_decision(
            run_with_target(TriageAction::React, "missing-agent", "handle this"),
            &envelope,
        )
        .await
        .expect_err("missing target agent should fail");
        assert!(err.to_string().contains("missing-agent"));

        sleep(Duration::from_millis(20)).await;
        let captured = seen.lock().await;
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
        let envelope = envelope("esc-escalate-fail");
        let _ = init_global(32);
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let seen = Arc::new(Mutex::new(Vec::<DomainEvent>::new()));
        let seen_handler = Arc::clone(&seen);
        let _handle = global()
            .unwrap()
            .on("triage-escalation-escalate-fail", move |event| {
                let seen = Arc::clone(&seen_handler);
                let cloned = event.clone();
                Box::pin(async move {
                    seen.lock().await.push(cloned);
                })
            });

        let err = apply_decision(
            run_with_target(TriageAction::Escalate, "missing-agent", "escalate this"),
            &envelope,
        )
        .await
        .expect_err("missing orchestrator target should fail");
        assert!(err.to_string().contains("missing-agent"));

        sleep(Duration::from_millis(20)).await;
        let captured = seen.lock().await;
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
