//! Route an [`EnrichedTask`] onto the agent's work surface.
//!
//! Every enriched task lands as a card on the dedicated `task-sources`
//! thread board (reusing the thread-scoped `todos` store). Sources with
//! the [`SourceTarget::AgentTodoProactive`] target additionally dispatch
//! a triage turn — the same `TriggerEnvelope` → `run_triage` →
//! `apply_decision` path Composio webhooks use — so an agent can start
//! working immediately. Triage's classifier (drop / acknowledge / react
//! / escalate) gates noise, and the proactive turn is held behind the
//! `scheduler_gate` capacity semaphore so background AI throttling is
//! respected.

use serde_json::json;

use crate::openhuman::agent::triage::{apply_decision, run_triage, TriageOutcome, TriggerEnvelope};
use crate::openhuman::config::Config;
use crate::openhuman::todos::ops::{
    add as todo_add, remove as todo_remove, BoardLocation, CardPatch,
};
use crate::openhuman::{scheduler_gate, todos};

use super::types::{EnrichedTask, SourceTarget, TaskSource};

/// Stable thread id whose board collects every ingested task.
pub const TASK_SOURCES_THREAD_ID: &str = "task-sources";

/// Route an enriched task: append a todo card, then (for proactive
/// sources) dispatch a triage turn. Returns the new card id on success.
pub async fn route_enriched(
    config: &Config,
    source: &TaskSource,
    enriched: &EnrichedTask,
    stale_card_id: Option<&str>,
) -> Result<String, String> {
    let card_id = add_card(config, source, enriched, stale_card_id)?;

    match source.target {
        SourceTarget::TodoOnly => {
            tracing::debug!(
                source_id = %source.id,
                external_id = %enriched.task.external_id,
                "[task_sources:route] todo-only target, card added (no agent turn)"
            );
            Ok(card_id)
        }
        SourceTarget::AgentTodoProactive => {
            dispatch_triage(source, enriched).await?;
            Ok(card_id)
        }
    }
}

/// Append a new card on the `task-sources` board, optionally removing a
/// stale card first (when an upstream task was edited and re-routed). Returns
/// the id of the newly created card.
///
/// Removing the stale card before adding the new one prevents duplicate board
/// entries from accumulating across edit cycles. If the stale card is already
/// gone (e.g. user manually removed it) the remove error is logged and
/// ignored so the fresh card still lands.
fn add_card(
    config: &Config,
    source: &TaskSource,
    enriched: &EnrichedTask,
    stale_card_id: Option<&str>,
) -> Result<String, String> {
    let location = BoardLocation::Thread {
        workspace_dir: config.workspace_dir.clone(),
        thread_id: TASK_SOURCES_THREAD_ID.to_string(),
    };

    // Remove stale card from the previous ingestion of this task (if any)
    // before creating the replacement, so the board never accumulates
    // duplicate cards for the same upstream item.
    if let Some(old_id) = stale_card_id {
        match todo_remove(&location, old_id) {
            Ok(_) => {
                tracing::debug!(
                    source_id = %source.id,
                    external_id = %enriched.task.external_id,
                    stale_card_id = %old_id,
                    "[task_sources:route] stale card removed before re-routing edited task"
                );
            }
            Err(e) => {
                // Not fatal: card may have been manually removed already.
                tracing::debug!(
                    source_id = %source.id,
                    external_id = %enriched.task.external_id,
                    stale_card_id = %old_id,
                    error = %e,
                    "[task_sources:route] stale card removal skipped (already gone?)"
                );
            }
        }
    }

    let task = &enriched.task;
    let label = provider_label(&task.provider);
    let content = format!("[{label}] {}", task.title.trim());

    let mut notes_parts: Vec<String> = Vec::new();
    if enriched.summary.trim() != task.title.trim() && !enriched.summary.trim().is_empty() {
        notes_parts.push(enriched.summary.trim().to_string());
    }
    if let Some(url) = task.url.as_deref().filter(|s| !s.trim().is_empty()) {
        notes_parts.push(url.trim().to_string());
    }
    let notes = if notes_parts.is_empty() {
        None
    } else {
        Some(notes_parts.join("\n"))
    };

    let snapshot = todo_add(
        &location,
        &content,
        CardPatch {
            notes,
            ..Default::default()
        },
    )
    .map_err(|e| format!("[task_sources:route] failed to add todo card: {e}"))?;

    // The newly created card is always the last one in the snapshot (add
    // appends at the end). Return its id for the dedup ledger.
    let new_card_id = snapshot
        .cards
        .last()
        .map(|c| c.id.clone())
        .ok_or_else(|| "[task_sources:route] add returned empty card list".to_string())?;

    tracing::debug!(
        external_id = %task.external_id,
        card_id = %new_card_id,
        cards = snapshot.cards.len(),
        "[task_sources:route] card added to task-sources board"
    );
    Ok(new_card_id)
}

/// Dispatch a triage turn for a proactive task, gated by scheduler
/// capacity. Card creation already happened; a gated-off or deferred
/// turn is non-fatal — the task still sits on the board.
async fn dispatch_triage(source: &TaskSource, enriched: &EnrichedTask) -> Result<(), String> {
    // Respect background-AI throttling. When the gate denies capacity
    // (Off / paused), we keep the card but skip the proactive turn.
    let Some(_permit) = scheduler_gate::wait_for_capacity().await else {
        tracing::info!(
            source_id = %source.id,
            "[task_sources:route] scheduler gate denied capacity; card added, agent turn skipped"
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
            apply_decision(run, &envelope)
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
                "[task_sources:route] triage deferred (card remains on board)"
            );
        }
    }
    Ok(())
}

/// Title-case a provider slug for display on the card.
fn provider_label(provider: &str) -> String {
    match provider {
        "github" => "GitHub".to_string(),
        "notion" => "Notion".to_string(),
        "linear" => "Linear".to_string(),
        "clickup" => "ClickUp".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

/// Read the current cards on the `task-sources` board. Used by tests and
/// callers that want to inspect routed work without an RPC round-trip.
pub fn board_cards(
    config: &Config,
) -> Result<Vec<crate::openhuman::agent::task_board::TaskBoardCard>, String> {
    let location = BoardLocation::Thread {
        workspace_dir: config.workspace_dir.clone(),
        thread_id: TASK_SOURCES_THREAD_ID.to_string(),
    };
    todos::ops::list(&location).map(|snap| snap.cards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_label_titlecases_known_and_unknown() {
        assert_eq!(provider_label("github"), "GitHub");
        assert_eq!(provider_label("clickup"), "ClickUp");
        assert_eq!(provider_label("asana"), "Asana");
        assert_eq!(provider_label(""), "");
    }
}
