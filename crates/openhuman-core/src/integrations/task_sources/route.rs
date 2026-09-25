//! Route an [`EnrichedTask`] into the agent.
//!
//! The ingestion ledger (`store.rs`) is the record of what was pulled from a
//! source. Sources with the [`SourceTarget::AgentTodoProactive`] target
//! dispatch a triage turn for each new task — the same `TriggerEnvelope` →
//! `run_triage` → `apply_decision` path Composio webhooks use — so an agent
//! can start working immediately; triage's classifier (drop / acknowledge /
//! react / escalate) gates noise, and the proactive turn is held behind the
//! `scheduler_gate` capacity semaphore so background AI throttling is
//! respected. [`SourceTarget::TodoOnly`] sources are collected into the ledger
//! and go no further.
//!
//! Tasks used to be mirrored as cards onto a `task-sources` thread board as
//! well. That board was rendered nowhere and the `todo` tool is now the
//! session's own list, so the mirror is gone; the ledger is the surface.

use serde_json::json;

use crate::agent::triage::{
    apply_decision, remote_trigger_origin, run_triage, TriageOutcome, TriggerEnvelope,
};
use crate::agent::turn_origin::with_origin;
use crate::config::Config;
use crate::cron::scheduler_gate;

use super::types::{EnrichedTask, SourceTarget, TaskSource};

/// Route an enriched task: for proactive sources dispatch a triage turn;
/// collect-only sources stop at the ledger the caller already wrote.
pub async fn route_enriched(
    _config: &Config,
    source: &TaskSource,
    enriched: &EnrichedTask,
) -> Result<(), String> {
    match source.target {
        SourceTarget::TodoOnly => {
            tracing::debug!(
                source_id = %source.id,
                external_id = %enriched.task.external_id,
                "[task_sources:route] collect-only target, no agent turn"
            );
            Ok(())
        }
        SourceTarget::AgentTodoProactive => dispatch_triage(source, enriched).await,
    }
}

/// Dispatch a triage turn for a proactive task, gated by scheduler
/// capacity. A gated-off or deferred turn is non-fatal — the task is already
/// in the ledger.
async fn dispatch_triage(source: &TaskSource, enriched: &EnrichedTask) -> Result<(), String> {
    // Respect background-AI throttling. When the gate denies capacity
    // (Off / paused), we keep the card but skip the proactive turn.
    let Some(_permit) = scheduler_gate::wait_for_capacity().await else {
        tracing::info!(
            source_id = %source.id,
            "[task_sources:route] scheduler gate denied capacity; agent turn skipped"
        );
        return Ok(());
    };

    let task = &enriched.task;
    let payload = json!({
        "task": task,
        "summary": enriched.summary,
        "agentPrompt": enriched.agent_prompt,
        "urgency": enriched.urgency,
        "url": task.url,
        "provider": task.provider,
        "sourceId": source.id,
    });

    let envelope = TriggerEnvelope::from_external(
        &format!("task_sources:{}", source.id),
        "external task ingested",
        payload,
    );

    let outcome = run_triage(&envelope)
        .await
        .map_err(|e| format!("[task_sources:route] triage evaluation failed: {e}"))?;

    match outcome {
        TriageOutcome::Decision(run) => {
            // Remote payload: the task arrived from an external board, so the
            // dispatch parks rather than running on a trust root (#5634).
            let origin = remote_trigger_origin(&envelope);
            with_origin(origin, apply_decision(run, &envelope))
                .await
                .map_err(|e| format!("[task_sources:route] apply_decision failed: {e}"))?;
            tracing::debug!(
                source_id = %source.id,
                external_id = %task.external_id,
                "[task_sources:route] triage decision applied"
            );
        }
        TriageOutcome::Deferred { reason, .. } => {
            tracing::debug!(
                source_id = %source.id,
                reason = %reason,
                "[task_sources:route] triage deferred (task stays in the ledger)"
            );
        }
        TriageOutcome::Terminal { reason } => {
            tracing::warn!(
                source_id = %source.id,
                external_id = %task.external_id,
                reason = %reason,
                "[task_sources:route] triage reached terminal state"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;
