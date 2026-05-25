//! High-level memory sync orchestration.
//!
//! This module owns the user-facing "sync my memory" workflow:
//!
//! 1. accept a manual or scheduled sync request
//! 2. emit coarse lifecycle events for UI visibility
//! 3. dispatch into [`crate::openhuman::memory_sync`] backends
//! 4. rely on `memory_store` + `memory_queue` + `memory_tree` backends to
//!    persist, enqueue, ingest, and seal the resulting data
//!
//! The low-level provider implementations live in `memory_sync/*`; this module
//! is the orchestration seam the `memory` domain presents to RPC/tools/UI.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::event_bus::{
    publish_global, subscribe_global, DomainEvent, EventHandler, SubscriptionHandle,
};

/// Why a sync run was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySyncTrigger {
    Manual,
    Cron,
}

impl MemorySyncTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Cron => "cron",
        }
    }
}

/// Coarse orchestration stages surfaced to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySyncStage {
    Requested,
    Fetching,
    Stored,
    Queued,
    Ingesting,
    Completed,
    Failed,
}

impl MemorySyncStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Fetching => "fetching",
            Self::Stored => "stored",
            Self::Queued => "queued",
            Self::Ingesting => "ingesting",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Publish a coarse sync lifecycle event for UI subscribers.
pub fn emit_sync_stage(
    trigger: MemorySyncTrigger,
    stage: MemorySyncStage,
    provider: Option<&str>,
    connection_id: Option<&str>,
    detail: Option<String>,
) {
    publish_global(DomainEvent::MemorySyncStageChanged {
        trigger: trigger.as_str().to_string(),
        stage: stage.as_str().to_string(),
        provider: provider.map(str::to_string),
        connection_id: connection_id.map(str::to_string),
        detail,
    });
}

static MEMORY_SYNC_FRONTEND_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register a lightweight bridge that translates lower-level ingestion events
/// into the coarse sync-stage stream the frontend consumes.
pub fn register_sync_stage_bridge() {
    if MEMORY_SYNC_FRONTEND_HANDLE.get().is_some() {
        return;
    }
    match subscribe_global(Arc::new(MemorySyncStageBridge)) {
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
}

struct MemorySyncStageBridge;

#[async_trait]
impl EventHandler for MemorySyncStageBridge {
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
                emit_sync_stage(
                    MemorySyncTrigger::Manual,
                    MemorySyncStage::Stored,
                    Some(provider),
                    None,
                    Some(format!(
                        "canonicalized {chunks_written} chunks from {source_id}"
                    )),
                );
                emit_sync_stage(
                    MemorySyncTrigger::Manual,
                    MemorySyncStage::Queued,
                    Some(provider),
                    None,
                    Some(format!("queued chunk extraction for {source_id}")),
                );
            }
            DomainEvent::MemoryIngestionStarted {
                document_id,
                namespace,
                queue_depth,
                ..
            } => {
                emit_sync_stage(
                    MemorySyncTrigger::Manual,
                    MemorySyncStage::Ingesting,
                    Some(namespace),
                    Some(document_id),
                    Some(format!("queue_depth={queue_depth}")),
                );
            }
            _ => {}
        }
    }
}
