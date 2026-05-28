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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, OnceLock};

    use crate::core::event_bus::{self, init_global, subscribe_global};

    fn test_mutex() -> &'static std::sync::Mutex<()> {
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[derive(Clone, Default)]
    struct StageCollector {
        events: Arc<Mutex<Vec<DomainEvent>>>,
    }

    #[async_trait]
    impl EventHandler for StageCollector {
        fn name(&self) -> &str {
            "memory::sync::tests::stage_collector"
        }

        fn domains(&self) -> Option<&[&str]> {
            Some(&["memory"])
        }

        async fn handle(&self, event: &DomainEvent) {
            if matches!(event, DomainEvent::MemorySyncStageChanged { .. }) {
                self.events.lock().unwrap().push(event.clone());
            }
        }
    }

    #[tokio::test]
    async fn document_canonicalized_emits_stored_and_queued_stages() {
        let _guard = test_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        init_global(event_bus::DEFAULT_CAPACITY);

        let collector = StageCollector::default();
        let _subscription =
            subscribe_global(Arc::new(collector.clone())).expect("event bus initialized");

        let bridge = MemorySyncStageBridge;
        bridge
            .handle(&DomainEvent::DocumentCanonicalized {
                source_id: "slack:workspace-1".into(),
                source_kind: "chat".into(),
                chunks_written: 3,
                chunk_ids: vec!["chunk-1".into()],
                canonicalized_at: 1_700_000_000.0,
                body_preview: None,
            })
            .await;

        tokio::task::yield_now().await;

        let stages: Vec<String> = collector
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                DomainEvent::MemorySyncStageChanged { stage, .. } => Some(stage.clone()),
                _ => None,
            })
            .collect();
        assert!(stages.contains(&"stored".to_string()));
        assert!(stages.contains(&"queued".to_string()));
    }

    #[tokio::test]
    async fn memory_ingestion_started_emits_ingesting_stage() {
        let _guard = test_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        init_global(event_bus::DEFAULT_CAPACITY);

        let collector = StageCollector::default();
        let _subscription =
            subscribe_global(Arc::new(collector.clone())).expect("event bus initialized");

        let bridge = MemorySyncStageBridge;
        bridge
            .handle(&DomainEvent::MemoryIngestionStarted {
                document_id: "doc-123".into(),
                title: "Vault Note".into(),
                namespace: "vault:v-1".into(),
                queue_depth: 2,
            })
            .await;

        tokio::task::yield_now().await;

        let ingesting = collector
            .events
            .lock()
            .unwrap()
            .iter()
            .find_map(|event| match event {
                DomainEvent::MemorySyncStageChanged {
                    stage,
                    provider,
                    connection_id,
                    detail,
                    ..
                } if stage == "ingesting" => {
                    Some((provider.clone(), connection_id.clone(), detail.clone()))
                }
                _ => None,
            })
            .expect("ingesting stage should be emitted");

        assert_eq!(ingesting.0.as_deref(), Some("vault:v-1"));
        assert_eq!(ingesting.1.as_deref(), Some("doc-123"));
        assert_eq!(ingesting.2.as_deref(), Some("queue_depth=2"));
    }
}
