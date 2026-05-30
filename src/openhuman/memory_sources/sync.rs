//! Per-source sync orchestration.
//!
//! Dispatches sync requests to the right backend based on source kind:
//! - Composio sources delegate to `memory_sync::composio::run_connection_sync`
//! - Folder/GitHub/RSS/WebPage sources walk items via the reader and
//!   ingest each one through `memory::ingest_pipeline::ingest_document`
//! - Twitter is a placeholder until credentials wiring lands
//!
//! Sync runs in a `tokio::spawn`-ed task so the RPC returns immediately
//! after queueing. Progress is published as `MemorySyncStageChanged`
//! events on the global bus and UI subscribers stream them per source id.

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::ingest_document;
use crate::openhuman::memory::sync::{emit_sync_stage, MemorySyncStage, MemorySyncTrigger};
use crate::openhuman::memory_sources::readers;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;
use crate::openhuman::memory_sync::composio::{self, SyncReason};

/// Trigger a sync for one source. Spawns work in the background and
/// returns immediately. Progress is published as `MemorySyncStageChanged`
/// events with `connection_id = Some(source.id)`.
pub async fn sync_source(source: MemorySourceEntry, config: Config) -> Result<(), String> {
    if !source.enabled {
        return Err(format!("source '{}' is disabled", source.id));
    }

    let source_id = source.id.clone();
    let kind_str = source.kind.as_str();

    tracing::debug!(
        source_id = %source_id,
        kind = %kind_str,
        "[memory_sources:sync] queueing sync"
    );

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Requested,
        Some(kind_str),
        Some(&source_id),
        Some(format!("sync requested for {} source", kind_str)),
    );

    // Outer spawn catches panics so a panic in the sync task is surfaced
    // as a tracing::error! log rather than silently dropping the join handle.
    tokio::spawn(async move {
        let source_id_for_panic = source.id.clone();
        let kind_for_panic = source.kind.as_str();
        let inner = tokio::spawn(async move {
            tracing::debug!(
                source_id = %source.id,
                kind = %source.kind.as_str(),
                "[memory_sources:sync] dispatching by kind"
            );
            let outcome = match source.kind {
                SourceKind::Composio => sync_composio(&source, config).await,
                SourceKind::Folder
                | SourceKind::GithubRepo
                | SourceKind::RssFeed
                | SourceKind::WebPage => sync_via_reader(&source, config).await,
                SourceKind::TwitterQuery => Err(
                    "Twitter sync not yet configured. Provide bearer token in settings."
                        .to_string(),
                ),
            };

            match outcome {
                Ok(items) => {
                    tracing::debug!(
                        source_id = %source.id,
                        kind = %source.kind.as_str(),
                        items = items,
                        "[memory_sources:sync] completed"
                    );
                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Completed,
                        Some(source.kind.as_str()),
                        Some(&source.id),
                        Some(format!("ingested {items} item(s)")),
                    );
                }
                Err(error) => {
                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Failed,
                        Some(source.kind.as_str()),
                        Some(&source.id),
                        Some(error.clone()),
                    );
                    tracing::warn!(
                        source_id = %source.id,
                        kind = %source.kind.as_str(),
                        error = %error,
                        "[memory_sources:sync] failed"
                    );
                }
            }
        });

        if let Err(join_err) = inner.await {
            if join_err.is_panic() {
                tracing::error!(
                    source_id = %source_id_for_panic,
                    kind = %kind_for_panic,
                    "[memory_sources:sync] sync task panicked"
                );
            }
        }
    });

    Ok(())
}

async fn sync_composio(source: &MemorySourceEntry, config: Config) -> Result<usize, String> {
    let connection_id = source
        .connection_id
        .as_deref()
        .ok_or("composio source missing connection_id")?;

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some("composio"),
        Some(&source.id),
        Some(format!("delegating to composio sync for {connection_id}")),
    );

    let outcome = composio::run_connection_sync(config, connection_id, SyncReason::Manual)
        .await
        .map_err(|e| format!("composio sync failed: {e}"))?;

    Ok(outcome.items_ingested)
}

async fn sync_via_reader(source: &MemorySourceEntry, config: Config) -> Result<usize, String> {
    let reader = readers::reader_for(&source.kind);

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some(source.kind.as_str()),
        Some(&source.id),
        Some("listing items".to_string()),
    );

    let items = reader.list_items(source, &config).await?;
    let total = items.len();
    tracing::debug!(
        source_id = %source.id,
        kind = %source.kind.as_str(),
        total = total,
        "[memory_sources:sync] reader.list_items returned items"
    );

    if total == 0 {
        return Ok(0);
    }

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Stored,
        Some(source.kind.as_str()),
        Some(&source.id),
        Some(format!("{total} item(s) discovered")),
    );

    let mut ingested = 0usize;
    for (idx, item) in items.iter().enumerate() {
        let content = match reader.read_item(source, &item.id, &config).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    item_id = %item.id,
                    error = %e,
                    "[memory_sources:sync] skipping item — read failed"
                );
                continue;
            }
        };

        let doc = DocumentInput {
            provider: format!("memory_sources:{}", source.kind.as_str()),
            title: content.title.clone(),
            body: content.body.clone(),
            modified_at: chrono::Utc::now(),
            source_ref: Some(format!("{}:{}", source.id, item.id)),
        };

        let composite_source_id = format!("mem_src:{}:{}", source.id, item.id);
        let tags = vec![
            "memory_sources".to_string(),
            source.kind.as_str().to_string(),
        ];

        match ingest_document(&config, &composite_source_id, "user", tags, doc).await {
            Ok(result) => {
                if !result.already_ingested {
                    ingested += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    item_id = %item.id,
                    error = %e,
                    "[memory_sources:sync] ingest failed for item"
                );
            }
        }

        // Emit progress every 5 items or at the end
        if (idx + 1) % 5 == 0 || idx + 1 == total {
            emit_sync_stage(
                MemorySyncTrigger::Manual,
                MemorySyncStage::Ingesting,
                Some(source.kind.as_str()),
                Some(&source.id),
                Some(format!("{}/{total} processed", idx + 1)),
            );
        }
    }

    Ok(ingested)
}
