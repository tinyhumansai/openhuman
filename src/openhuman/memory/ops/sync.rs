//! Memory-sync RPC handlers and ingestion-status reporting.
//!
//! Sync RPCs publish `DomainEvent::MemorySyncRequested` on the global event
//! bus — they are fire-and-forget hooks for future ingestion subscribers.

use crate::rpc::RpcOutcome;

/// Parameters for `memory_sync_channel`.
#[derive(Debug, serde::Deserialize)]
pub struct SyncChannelParams {
    pub channel_id: String,
}

/// Result returned by `memory_sync_channel`.
#[derive(Debug, serde::Serialize)]
pub struct SyncChannelResult {
    pub requested: bool,
    pub channel_id: String,
}

/// Result returned by `memory_sync_all`.
#[derive(Debug, serde::Serialize)]
pub struct SyncAllResult {
    pub requested: bool,
}

/// Result returned by `memory_ingestion_status`. Mirrors
/// [`crate::openhuman::memory::IngestionStatusSnapshot`] but is the public RPC
/// shape — the indirection keeps internal renames from breaking the wire
/// contract.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IngestionStatusResult {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_namespace: Option<String>,
    pub queue_depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success: Option<bool>,
}

/// Request a memory sync for a specific channel.
///
/// Ingestion in OpenHuman is listener/webhook-driven — there is no per-provider
/// pull mechanism yet. This RPC publishes `DomainEvent::MemorySyncRequested` so
/// that future ingestion subscribers can react to an explicit pull request.
/// The event is fire-and-forget; the caller receives confirmation that the
/// request was published, not that ingestion ran.
pub async fn memory_sync_channel(
    params: SyncChannelParams,
) -> Result<RpcOutcome<SyncChannelResult>, String> {
    // `channel_id` is a user/context identifier — keep it out of normal logs.
    tracing::info!("[memory.sync] memory_sync_channel: entry");
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::MemorySyncRequested {
            channel_id: Some(params.channel_id.clone()),
        },
    );
    tracing::debug!("[memory.sync] memory_sync_channel: MemorySyncRequested published");
    Ok(RpcOutcome::new(
        SyncChannelResult {
            requested: true,
            channel_id: params.channel_id,
        },
        vec![],
    ))
}

/// Request a memory sync for all channels.
///
/// Publishes `DomainEvent::MemorySyncRequested { channel_id: None }` on the
/// global event bus. No consumers exist yet — this is a hook for future
/// ingestion subscribers.
pub async fn memory_sync_all() -> Result<RpcOutcome<SyncAllResult>, String> {
    tracing::info!("[memory.sync] memory_sync_all: entry");
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::MemorySyncRequested { channel_id: None },
    );
    tracing::debug!("[memory.sync] memory_sync_all: MemorySyncRequested(all) published");
    Ok(RpcOutcome::new(SyncAllResult { requested: true }, vec![]))
}

/// Returns the current memory-ingestion status: whether a job is running, the
/// in-flight document, queue depth, and the most recent completion. Read-only,
/// safe to poll.
pub async fn memory_ingestion_status() -> Result<RpcOutcome<IngestionStatusResult>, String> {
    let snapshot = match crate::openhuman::memory::global::client_if_ready() {
        Some(c) => c.ingestion_state().snapshot(),
        // Memory not yet initialised — report idle, no in-flight job.
        None => Default::default(),
    };
    Ok(RpcOutcome::new(
        IngestionStatusResult {
            running: snapshot.running,
            current_document_id: snapshot.current_document_id,
            current_title: snapshot.current_title,
            current_namespace: snapshot.current_namespace,
            queue_depth: snapshot.queue_depth,
            last_completed_at: snapshot.last_completed_at,
            last_document_id: snapshot.last_document_id,
            last_success: snapshot.last_success,
        },
        vec![],
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, OnceLock};

    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    use crate::core::event_bus::{self, DomainEvent, EventHandler};

    fn test_mutex() -> &'static std::sync::Mutex<()> {
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn ensure_memory_client() -> crate::openhuman::memory::MemoryClientRef {
        static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
        let workspace = WORKSPACE.get_or_init(|| {
            let tmp = TempDir::new().expect("tempdir");
            let path = tmp.path().join("workspace");
            std::fs::create_dir_all(&path).expect("workspace dir");
            std::mem::forget(tmp);
            path
        });
        crate::openhuman::memory::global::init(workspace.clone()).expect("init memory client")
    }

    struct ChannelCapture {
        tx: mpsc::UnboundedSender<Option<String>>,
    }

    #[async_trait]
    impl EventHandler for ChannelCapture {
        fn name(&self) -> &str {
            "memory::ops::sync::tests::capture"
        }

        fn domains(&self) -> Option<&[&str]> {
            Some(&["memory"])
        }

        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::MemorySyncRequested { channel_id } = event {
                let _ = self.tx.send(channel_id.clone());
            }
        }
    }

    #[test]
    fn sync_channel_params_deserialize_channel_id() {
        let params: SyncChannelParams =
            serde_json::from_value(json!({"channel_id": "channel-1"})).unwrap();
        assert_eq!(params.channel_id, "channel-1");
    }

    #[test]
    fn ingestion_status_result_default_is_idle() {
        let status = IngestionStatusResult::default();
        assert!(!status.running);
        assert!(status.current_document_id.is_none());
        assert!(status.current_title.is_none());
        assert!(status.current_namespace.is_none());
        assert_eq!(status.queue_depth, 0);
        assert!(status.last_completed_at.is_none());
        assert!(status.last_document_id.is_none());
        assert!(status.last_success.is_none());
    }

    #[test]
    fn sync_result_structs_serialize_expected_fields() {
        let one = serde_json::to_value(SyncChannelResult {
            requested: true,
            channel_id: "abc".into(),
        })
        .unwrap();
        assert_eq!(one, json!({"requested": true, "channel_id": "abc"}));

        let all = serde_json::to_value(SyncAllResult { requested: true }).unwrap();
        assert_eq!(all, json!({"requested": true}));
    }

    #[tokio::test]
    async fn memory_sync_channel_publishes_targeted_event() {
        let _guard = test_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        event_bus::init_global(event_bus::DEFAULT_CAPACITY);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _subscription = event_bus::subscribe_global(Arc::new(ChannelCapture { tx }))
            .expect("global bus should be initialized");

        let outcome = memory_sync_channel(SyncChannelParams {
            channel_id: "channel-123".into(),
        })
        .await
        .expect("memory_sync_channel");
        assert!(outcome.value.requested);
        assert_eq!(outcome.value.channel_id, "channel-123");

        let received = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("event should arrive before timeout")
            .expect("sender should still be connected");
        assert_eq!(received.as_deref(), Some("channel-123"));
    }

    #[tokio::test]
    async fn memory_sync_all_publishes_broadcast_event() {
        let _guard = test_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        event_bus::init_global(event_bus::DEFAULT_CAPACITY);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _subscription = event_bus::subscribe_global(Arc::new(ChannelCapture { tx }))
            .expect("global bus should be initialized");

        let outcome = memory_sync_all().await.expect("memory_sync_all");
        assert!(outcome.value.requested);

        let received = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("event should arrive before timeout")
            .expect("sender should still be connected");
        assert!(
            received.is_none(),
            "sync-all should publish channel_id=None"
        );
    }

    #[tokio::test]
    async fn memory_ingestion_status_reflects_initialized_client_snapshot() {
        let _guard = test_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let client = ensure_memory_client();
        let state = client.ingestion_state();

        state.enqueue();
        state.mark_running("doc-sync", "Sync Title", "sync-test");

        let status = memory_ingestion_status()
            .await
            .expect("memory_ingestion_status")
            .value;

        assert!(status.running);
        assert_eq!(status.current_document_id.as_deref(), Some("doc-sync"));
        assert_eq!(status.current_title.as_deref(), Some("Sync Title"));
        assert_eq!(status.current_namespace.as_deref(), Some("sync-test"));
        assert_eq!(status.queue_depth, 1);

        state.dequeue();
        state.mark_completed("doc-sync", true, 12345);
    }
}
