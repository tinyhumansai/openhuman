//! The `core::bus` subscriber half of the memory sync-stage stream.
//!
//! [`tinymemory_api::sync_events`] owns the vocabulary (`MemorySyncTrigger`,
//! `MemorySyncStage`) and the *emit* side, which goes out through the seam's
//! event sink. What lives here is the *subscribe* side: the bridge that
//! translates lower-level ingestion events into the coarse stage stream the
//! frontend consumes, and the post-sync trigger that kicks off batch embedding.
//!
//! Subscribers are host surface by the tinymemory README's split — they name
//! `DomainEvent`, which is OpenHuman's own vocabulary spanning agents,
//! channels, cron and tools, and `BUS`, which the engine crate has no business
//! knowing about.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;
use tinymemory_api::sync_events::{
    emit_sync_stage, extract_mem_src_id, MemorySyncStage, MemorySyncTrigger,
};

static MEMORY_SYNC_FRONTEND_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static MEMORY_SYNC_EMBED_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register a lightweight bridge that translates lower-level ingestion events
/// into the coarse sync-stage stream the frontend consumes, and a post-sync
/// embed trigger that kicks off batch embedding after sync completion.
pub fn register_sync_stage_bridge(config: &Config) {
    if MEMORY_SYNC_FRONTEND_HANDLE.get().is_some() {
        return;
    }
    match BUS.subscribe(Arc::new(MemorySyncStageBridge)) {
        Some(handle) => {
            let _ = MEMORY_SYNC_FRONTEND_HANDLE.set(handle);
            log::debug!("[event_bus] memory sync stage bridge registered");
        }
        None => {
            log::warn!(
                "[event_bus] failed to register memory sync stage bridge — bus not initialized"
            );
        }
    }

    // The process's memory of the stream the bridge feeds: which sources are
    // in flight right now, for the status list (openhuman#6019).
    super::sync_activity::register();

    // Trigger batch embedding when a sync completes. Extract no longer embeds
    // inline — the backfill pass picks up all un-embedded chunks in large
    // batches (up to 1000 items per API call).
    if MEMORY_SYNC_EMBED_HANDLE.get().is_none() {
        if let Some(handle) = BUS.subscribe(Arc::new(SyncCompleteEmbedTrigger {
            config: config.clone(),
        })) {
            let _ = MEMORY_SYNC_EMBED_HANDLE.set(handle);
            log::debug!("[event_bus] sync-complete embed trigger registered");
        }
    }
}

/// Triggers a `ReembedBackfill` chain when a sync completes so that all
/// chunks admitted during the sync get their embeddings in one large batch
/// pass (up to 1000 items per API call, ~1M tokens).
struct SyncCompleteEmbedTrigger {
    config: Config,
}

#[async_trait]
impl EventHandler<DomainEvent> for SyncCompleteEmbedTrigger {
    fn name(&self) -> &str {
        "memory::sync_complete_embed_trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::MemorySyncStageChanged { stage, .. } = event {
            if stage == "completed" {
                log::debug!("[memory-sync] sync completed — triggering batch embedding backfill");
                crate::openhuman::memory::ops::maintenance::reembed_best_effort(
                    &self.config,
                    "sync event",
                )
                .await;
            }
        }
    }
}

struct MemorySyncStageBridge;

#[async_trait]
impl EventHandler<DomainEvent> for MemorySyncStageBridge {
    fn name(&self) -> &str {
        "memory::sync_stage_bridge"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::DocumentCanonicalized {
                source_id,
                source_kind,
                chunks_written,
                ..
            } => {
                let provider = source_id.split(':').next().unwrap_or(source_kind);
                // Extract the memory-source id from the composite "mem_src:<source_id>:<item>"
                // format used by the reader-based ingest path. For non-memory-source syncs
                // (e.g. "slack:workspace-1") this returns None and source_id stays None.
                let mem_src_id = extract_mem_src_id(source_id);
                log::debug!(
                    "[memory-sync] bridge: DocumentCanonicalized source_id={} mem_src_id={:?}",
                    source_id,
                    mem_src_id
                );
                emit_sync_stage(
                    MemorySyncTrigger::Manual,
                    MemorySyncStage::Stored,
                    Some(provider),
                    None,
                    Some(format!(
                        "canonicalized {chunks_written} chunks from {source_id}"
                    )),
                    mem_src_id,
                );
                emit_sync_stage(
                    MemorySyncTrigger::Manual,
                    MemorySyncStage::Queued,
                    Some(provider),
                    None,
                    Some(format!("queued chunk extraction for {source_id}")),
                    mem_src_id,
                );
            }
            DomainEvent::MemoryIngestionStarted {
                document_id,
                namespace,
                queue_depth,
                ..
            } => {
                // The document_id for reader-based ingest is "mem_src:<source_id>:<item_id>".
                // Extract the memory-source id so the frontend can match the row.
                // document_id keeps carrying its original value in connection_id for
                // downstream consumers (dedup keys, audit). We only ADD source_id here.
                let mem_src_id = extract_mem_src_id(document_id);
                log::debug!(
                    "[memory-sync] bridge: MemoryIngestionStarted document_id={} mem_src_id={:?}",
                    document_id,
                    mem_src_id
                );
                emit_sync_stage(
                    MemorySyncTrigger::Manual,
                    MemorySyncStage::Ingesting,
                    Some(namespace),
                    Some(document_id),
                    Some(format!("queue_depth={queue_depth}")),
                    mem_src_id,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "sync_events_bridge_tests.rs"]
mod tests;
