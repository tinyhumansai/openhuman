//! The fetch → dedup → enrich → route pipeline for one task source.
//!
//! [`run_source_once`] is the single entry point shared by the periodic
//! poll, the manual `task_sources_fetch` RPC, and the
//! connection-created bus hook. It is intentionally infallible at the
//! call boundary: any error is captured into [`FetchOutcome::error`] so
//! the scheduler loop never unwinds.

use chrono::Utc;
use std::collections::HashSet;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;

use super::types::{FetchOutcome, FetchReason, TaskSource};
use super::{enrich, filter, route, store};

/// Run a single fetch pass over `source`. Captures errors into the
/// returned [`FetchOutcome`] rather than propagating them.
pub async fn run_source_once(
    config: &Config,
    source: &TaskSource,
    reason: FetchReason,
) -> FetchOutcome {
    let mut outcome = FetchOutcome {
        source_id: source.id.clone(),
        provider: source.provider.as_str().to_string(),
        ..Default::default()
    };

    tracing::info!(
        source_id = %source.id,
        provider = %source.provider.as_str(),
        reason = reason.as_str(),
        "[task_sources:pipeline] fetch pass starting"
    );

    match run_inner(config, source, reason, &mut outcome).await {
        Ok(()) => {
            let status = format!(
                "fetched {} routed {} dupes {} pruned {}",
                outcome.fetched, outcome.routed, outcome.skipped_dupe, outcome.pruned
            );
            let _ = store::record_fetch(config, &source.id, Utc::now(), reason, &status);
            BUS.publish(DomainEvent::TaskSourceFetched {
                source_id: source.id.clone(),
                provider: outcome.provider.clone(),
                fetched: outcome.fetched,
                routed: outcome.routed,
                skipped: outcome.skipped_dupe,
            });
            tracing::info!(
                source_id = %source.id,
                fetched = outcome.fetched,
                routed = outcome.routed,
                skipped_dupe = outcome.skipped_dupe,
                pruned = outcome.pruned,
                "[task_sources:pipeline] fetch pass complete"
            );
        }
        Err(e) => {
            tracing::warn!(
                source_id = %source.id,
                error = %e,
                "[task_sources:pipeline] fetch pass failed"
            );
            let _ = store::record_fetch(
                config,
                &source.id,
                Utc::now(),
                reason,
                &format!("error: {e}"),
            );
            BUS.publish(DomainEvent::TaskSourceFetchFailed {
                source_id: source.id.clone(),
                provider: outcome.provider.clone(),
                error: e.clone(),
            });
            outcome.error = Some(e);
        }
    }

    outcome
}

/// `ComposioProvider::fetch_tasks` — the per-toolkit "read a filtered set of
/// work items as structured `NormalizedTask`s" call this pipeline used to
/// make through the engine's provider registry (`get_provider(toolkit)`) —
/// was deleted outright by tinymemory v1.13.4 along with the rest of the
/// in-process Composio pipeline (72 files, ~18.3k lines), with no
/// replacement upstream.
///
/// Unlike the record-sync path (`ConnectorRecordBatch` via the
/// `tinyconnectors` module's `Sync` member, which this domain's sibling
/// `memory::sync::composio` now uses through `run_sync_pass`), tinyconnectors
/// exposes no structured task-fetch surface — only `Execute` (run one named
/// action) and `Sync` (memory records shaped for ingestion, not board
/// items). Reimplementing `fetch_tasks` faithfully means re-deriving each
/// toolkit's action selection and response-shape parsing (Notion's
/// `NOTION_QUERY_DATABASE` vs `NOTION_FETCH_DATA`, GitHub's issue listing,
/// Linear's/ClickUp's task queries, …) against `Execute` from scratch, which
/// is real per-provider work rather than a seam migration.
///
/// Rather than half-porting some toolkits' response parsing (unverifiable
/// without the toolkits' actual API responses to test against) and silently
/// dropping the rest, this refuses cleanly for every toolkit — the same
/// shape of decision `refuse_composio_dispatch` makes in the engine for the
/// composio-connection sync path it could not carry forward either. Every
/// other stage of this pipeline (dedup, enrichment, routing, storage,
/// reconciliation) is untouched and would work the moment this returns real
/// tasks.
fn fetch_tasks_unavailable(
    source: &TaskSource,
    _filter: &tinymemory_api::composio::TaskFetchFilter,
) -> Result<Vec<tinymemory_api::composio::NormalizedTask>, String> {
    Err(format!(
        "task_sources fetch for toolkit '{}' is unavailable: tinymemory v1.13.4 deleted \
         ComposioProvider::fetch_tasks with no replacement, and the tinyconnectors module \
         exposes no structured task-fetch surface to reimplement it against",
        source.provider.as_str()
    ))
}

async fn run_inner(
    config: &Config,
    source: &TaskSource,
    _reason: FetchReason,
    outcome: &mut FetchOutcome,
) -> Result<(), String> {
    let fetch_filter = filter::to_fetch_filter(&source.filter, source.max_tasks_per_fetch);
    let tasks = fetch_tasks_unavailable(source, &fetch_filter)?;
    outcome.fetched = tasks.len();
    let current_external_ids: HashSet<String> =
        tasks.iter().map(|task| task.external_id.clone()).collect();

    for mut task in tasks {
        // Stamp the originating source before dedup / enrichment.
        task.source_id = source.id.clone();

        let hash = store::content_hash(&task);
        if store::is_ingested(config, &source.id, &task.external_id, &hash)
            .map_err(|e| format!("dedup check failed: {e}"))?
        {
            // Dedup is scoped to THIS source (`WHERE source_id AND external_id`),
            // so the same external_id under a different source would NOT hit
            // here — it dedups per-source, never cross-source.
            tracing::debug!(
                source_id = %source.id,
                provider = %source.provider.as_str(),
                external_id = %task.external_id,
                content_hash = %hash,
                "[task_sources:dedup] skip — already ingested under THIS source with same content_hash (per-source, unchanged)"
            );
            outcome.skipped_dupe += 1;
            continue;
        }

        // Look up the stale card id (if any) before enrichment so we can
        // remove the old board card when re-routing an edited upstream task.
        let stale_card_id = store::get_card_id(config, &source.id, &task.external_id)
            .map_err(|e| format!("get_card_id failed: {e}"))?;

        tracing::debug!(
            source_id = %source.id,
            provider = %source.provider.as_str(),
            external_id = %task.external_id,
            content_hash = %hash,
            edited = stale_card_id.is_some(),
            "[task_sources:dedup] route — not a dupe for this source ({})",
            if stale_card_id.is_some() {
                "content changed since last ingest → re-route, replace stale card"
            } else {
                "new external_id for this source"
            }
        );

        let enriched = enrich::enrich_task(task);

        // Route first; only mark ingested on success so a routing
        // failure retries on the next pass instead of being silently
        // dropped.
        let new_card_id = match route::route_enriched(
            config,
            source,
            &enriched,
            stale_card_id.as_deref(),
        )
        .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(
                    source_id = %source.id,
                    external_id = %enriched.task.external_id,
                    error = %e,
                    "[task_sources:pipeline] routing failed (will retry next pass)"
                );
                continue;
            }
        };

        store::mark_ingested(config, &source.id, &enriched.task, &new_card_id)
            .map_err(|e| format!("mark_ingested failed: {e}"))?;
        BUS.publish(DomainEvent::TaskSourceTaskIngested {
            source_id: source.id.clone(),
            provider: enriched.task.provider.clone(),
            external_id: enriched.task.external_id.clone(),
            title: enriched.task.title.clone(),
            urgency: enriched.urgency,
        });
        outcome.routed += 1;
    }

    outcome.pruned = reconcile_missing_tasks(config, source, &current_external_ids).await?;

    Ok(())
}

async fn reconcile_missing_tasks(
    config: &Config,
    source: &TaskSource,
    current_external_ids: &HashSet<String>,
) -> Result<usize, String> {
    let ingested = store::list_ingested_refs(config, &source.id)
        .map_err(|e| format!("list_ingested_refs failed: {e}"))?;
    let mut pruned = 0usize;

    for item in ingested {
        if current_external_ids.contains(&item.external_id) {
            continue;
        }

        if let Some(card_id) = item.card_id.as_deref().filter(|id| !id.trim().is_empty()) {
            route::remove_card(config, card_id).await.map_err(|e| {
                format!(
                    "remove stale card '{}' for source '{}' external task '{}': {e}",
                    card_id, source.id, item.external_id
                )
            })?;
        }

        if store::remove_ingested(config, &source.id, &item.external_id)
            .map_err(|e| format!("remove_ingested failed: {e}"))?
        {
            pruned += 1;
            tracing::debug!(
                source_id = %source.id,
                provider = %source.provider.as_str(),
                external_id = %item.external_id,
                "[task_sources:pipeline] pruned task absent from latest source fetch"
            );
        }
    }

    Ok(pruned)
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
