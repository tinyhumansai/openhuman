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

use super::types::{EnrichedTask, FilterSpec, SourceTarget, TaskSource};

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
            dispatch_triage(config, source, enriched, &card_id).await?;
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

    // Objective: the bare upstream title (the card `content`/title is the
    // `[provider] title` display form; the objective is the clean goal the
    // executing agent works toward).
    let objective = {
        let t = task.title.trim();
        (!t.is_empty()).then(|| t.to_string())
    };

    // Stamp the source identifiers the downstream dispatcher / write-back
    // needs (provider + repo + issue id + url) plus the enrichment urgency
    // used for prioritisation. This is the only writer of `source_metadata`.
    let source_metadata = build_source_metadata(source, enriched);

    // G7: pre-assign the card to the source's configured executor so the
    // dispatcher runs it deterministically (no LLM router). Unset → unassigned.
    let assigned_agent = source
        .assigned_executor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let snapshot = todo_add(
        &location,
        &content,
        CardPatch {
            notes,
            objective,
            assigned_agent,
            source_metadata: Some(source_metadata),
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

/// Build the card's `source_metadata` from the originating source + task:
/// the provider/repo/issue identifiers a later dispatcher or external
/// write-back needs to address the upstream item, plus the enrichment
/// urgency used to prioritise pickup. Repo is only present for GitHub
/// sources (the other providers don't carry a repo concept).
fn build_source_metadata(source: &TaskSource, enriched: &EnrichedTask) -> serde_json::Value {
    let task = &enriched.task;
    let mut meta = json!({
        "provider": task.provider,
        "source_id": source.id,
        "external_id": task.external_id,
        "urgency": enriched.urgency,
    });
    if let Some(url) = task.url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        meta["url"] = json!(url);
    }
    if let FilterSpec::Github {
        repo: Some(repo), ..
    } = &source.filter
    {
        let repo = repo.trim();
        if !repo.is_empty() {
            meta["repo"] = json!(repo);
        }
    }
    meta
}

/// Dispatch a triage turn for a proactive task, gated by scheduler
/// capacity. Card creation already happened; a gated-off or deferred
/// turn is non-fatal — the task still sits on the board.
async fn dispatch_triage(
    config: &Config,
    source: &TaskSource,
    enriched: &EnrichedTask,
    card_id: &str,
) -> Result<(), String> {
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

    // Link the envelope to the board card so triage's escalation arm routes
    // it through the deterministic dispatcher (claim → autonomous run →
    // write-back) instead of the one-shot triage sub-agent.
    let location = BoardLocation::Thread {
        workspace_dir: config.workspace_dir.clone(),
        thread_id: TASK_SOURCES_THREAD_ID.to_string(),
    };
    let envelope = TriggerEnvelope::from_external(
        &format!("task_sources:{}", source.id),
        "external task ingested",
        payload,
    )
    .with_task_card(card_id.to_string(), location);

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
    use crate::openhuman::task_sources::types::ProviderSlug;
    use crate::openhuman::task_sources::NormalizedTask;
    use chrono::Utc;

    #[test]
    fn provider_label_titlecases_known_and_unknown() {
        assert_eq!(provider_label("github"), "GitHub");
        assert_eq!(provider_label("clickup"), "ClickUp");
        assert_eq!(provider_label("asana"), "Asana");
        assert_eq!(provider_label(""), "");
    }

    fn github_source(repo: Option<&str>) -> TaskSource {
        TaskSource {
            id: "ts-1".into(),
            provider: ProviderSlug::Github,
            connection_id: None,
            name: None,
            enabled: true,
            filter: FilterSpec::Github {
                repo: repo.map(str::to_string),
                labels: vec![],
                assignee_is_me: true,
                state: None,
                extra: json!({}),
            },
            interval_secs: 1800,
            target: SourceTarget::AgentTodoProactive,
            max_tasks_per_fetch: 25,
            assigned_executor: None,
            created_at: Utc::now(),
            last_fetch_at: None,
            last_status: None,
        }
    }

    fn enriched(external_id: &str, url: Option<&str>, urgency: f32) -> EnrichedTask {
        let task = NormalizedTask {
            external_id: external_id.into(),
            provider: "github".into(),
            title: "Fix the bug".into(),
            url: url.map(str::to_string),
            ..Default::default()
        };
        EnrichedTask {
            task,
            summary: "Fix the bug".into(),
            urgency,
            linked_people: vec![],
            linked_memory_ids: vec![],
            agent_prompt: "do it".into(),
            enriched_at: Utc::now(),
        }
    }

    #[test]
    fn source_metadata_carries_github_repo_and_identifiers() {
        let src = github_source(Some("octo/repo"));
        let e = enriched("123", Some("https://github.com/octo/repo/issues/123"), 0.7);
        let meta = build_source_metadata(&src, &e);
        assert_eq!(meta["provider"], json!("github"));
        assert_eq!(meta["source_id"], json!("ts-1"));
        assert_eq!(meta["external_id"], json!("123"));
        assert_eq!(meta["repo"], json!("octo/repo"));
        assert_eq!(
            meta["url"],
            json!("https://github.com/octo/repo/issues/123")
        );
        let urgency = meta["urgency"].as_f64().expect("urgency is a number");
        assert!((urgency - 0.7).abs() < 1e-6, "urgency was {urgency}");
    }

    #[test]
    fn source_metadata_omits_absent_repo_and_url() {
        let src = github_source(None);
        let e = enriched("9", None, 0.4);
        let meta = build_source_metadata(&src, &e);
        assert!(meta.get("repo").is_none());
        assert!(meta.get("url").is_none());
        assert_eq!(meta["external_id"], json!("9"));
        let urgency = meta["urgency"].as_f64().expect("urgency is a number");
        assert!((urgency - 0.4).abs() < 1e-6, "urgency was {urgency}");
    }

    fn temp_config() -> (tempfile::TempDir, Config) {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        (tmp, config)
    }

    #[test]
    fn add_card_stamps_objective_assigned_agent_and_metadata() {
        let (_tmp, config) = temp_config();
        let mut src = github_source(Some("octo/repo"));
        // Whitespace around the executor must be trimmed into assigned_agent.
        src.assigned_executor = Some("  agent-x  ".into());
        let e = enriched("123", Some("https://github.com/octo/repo/issues/123"), 0.7);

        add_card(&config, &src, &e, None).expect("add_card succeeds");

        let cards = board_cards(&config).expect("board_cards");
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        // Display title is the `[provider] title` form; objective is the bare title.
        assert_eq!(card.title, "[GitHub] Fix the bug");
        assert_eq!(card.objective.as_deref(), Some("Fix the bug"));
        assert_eq!(card.assigned_agent.as_deref(), Some("agent-x"));
        let meta = card
            .source_metadata
            .as_ref()
            .expect("source_metadata present");
        assert_eq!(meta["external_id"], json!("123"));
        assert_eq!(meta["repo"], json!("octo/repo"));
    }

    #[test]
    fn add_card_drops_whitespace_only_assigned_executor() {
        let (_tmp, config) = temp_config();
        let mut src = github_source(None);
        src.assigned_executor = Some("   ".into());
        let e = enriched("9", None, 0.4);

        add_card(&config, &src, &e, None).expect("add_card succeeds");

        let cards = board_cards(&config).expect("board_cards");
        assert_eq!(cards.len(), 1);
        assert!(
            cards[0].assigned_agent.is_none(),
            "whitespace-only executor should not assign the card"
        );
    }

    #[test]
    fn source_metadata_has_no_repo_for_non_github_provider() {
        let mut src = github_source(Some("octo/repo"));
        // A non-GitHub filter carries no repo concept.
        src.provider = ProviderSlug::Linear;
        src.filter = FilterSpec::Linear {
            team_id: None,
            assignee_is_me: true,
            state: None,
            extra: json!({}),
        };
        let e = enriched("LIN-5", None, 0.5);
        let meta = build_source_metadata(&src, &e);
        assert!(meta.get("repo").is_none());
        assert_eq!(meta["source_id"], json!("ts-1"));
    }
}
