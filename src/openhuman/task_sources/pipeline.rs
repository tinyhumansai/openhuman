//! The fetch → dedup → enrich → route pipeline for one task source.
//!
//! [`run_source_once`] is the single entry point shared by the periodic
//! poll, the manual `task_sources_fetch` RPC, and the
//! connection-created bus hook. It is intentionally infallible at the
//! call boundary: any error is captured into [`FetchOutcome::error`] so
//! the scheduler loop never unwinds.

use std::sync::Arc;

use chrono::Utc;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::{get_provider, ProviderContext};

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
                "fetched {} routed {} dupes {}",
                outcome.fetched, outcome.routed, outcome.skipped_dupe
            );
            let _ = store::record_fetch(config, &source.id, Utc::now(), reason, &status);
            publish_global(DomainEvent::TaskSourceFetched {
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
            publish_global(DomainEvent::TaskSourceFetchFailed {
                source_id: source.id.clone(),
                provider: outcome.provider.clone(),
                error: e.clone(),
            });
            outcome.error = Some(e);
        }
    }

    outcome
}

async fn run_inner(
    config: &Config,
    source: &TaskSource,
    _reason: FetchReason,
    outcome: &mut FetchOutcome,
) -> Result<(), String> {
    let provider = get_provider(source.provider.as_str()).ok_or_else(|| {
        format!(
            "no native provider registered for '{}'",
            source.provider.as_str()
        )
    })?;

    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: source.provider.as_str().to_string(),
        connection_id: source.connection_id.clone(),
    };

    let fetch_filter = filter::to_fetch_filter(&source.filter, source.max_tasks_per_fetch);
    let tasks = provider.fetch_tasks(&ctx, &fetch_filter).await?;
    outcome.fetched = tasks.len();

    for mut task in tasks {
        // Stamp the originating source before dedup / enrichment.
        task.source_id = source.id.clone();

        let hash = store::content_hash(&task);
        if store::is_ingested(config, &source.id, &task.external_id, &hash)
            .map_err(|e| format!("dedup check failed: {e}"))?
        {
            outcome.skipped_dupe += 1;
            continue;
        }

        // Look up the stale card id (if any) before enrichment so we can
        // remove the old board card when re-routing an edited upstream task.
        let stale_card_id = store::get_card_id(config, &source.id, &task.external_id)
            .map_err(|e| format!("get_card_id failed: {e}"))?;

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
        publish_global(DomainEvent::TaskSourceTaskIngested {
            source_id: source.id.clone(),
            provider: enriched.task.provider.clone(),
            external_id: enriched.task.external_id.clone(),
            title: enriched.task.title.clone(),
            urgency: enriched.urgency,
        });
        outcome.routed += 1;
    }

    Ok(())
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
