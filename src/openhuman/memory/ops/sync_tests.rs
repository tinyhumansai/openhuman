use std::sync::{Arc, OnceLock};

use super::*;
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;

fn test_mutex() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// A config on its own temp workspace with a driver bound that reports
/// `queue` verbatim.
///
/// The ingestion-status handler reads through the contract now, and the
/// real driver is a compiled module a unit test cannot load — so without a
/// binding installed the workspace resolves to the null driver and every
/// count answers zero, which is exactly the failure mode this handler was
/// fixed for. See `binding::FixedDiagnostics`.
fn bind_queue(
    queue: crate::openhuman::memory::api::provider::types::QueueStats,
) -> (tempfile::TempDir, Config) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &config.workspace_dir,
        &config.subsystems.memory,
        Default::default(),
        queue,
    );
    (tmp, config)
}

struct ChannelCapture {
    tx: mpsc::UnboundedSender<Option<String>>,
}

#[async_trait]
impl EventHandler<DomainEvent> for ChannelCapture {
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
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = crate::core::bus::init().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _subscription = BUS
        .subscribe(Arc::new(ChannelCapture { tx }))
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
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let _guard = test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = crate::core::bus::init().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _subscription = BUS
        .subscribe(Arc::new(ChannelCapture { tx }))
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

/// The mapping from the driver's `QueueStats` onto the RPC shape, including
/// the fields the contract does not carry.
///
/// This replaces a test that drove the in-process engine's `IngestionState`
/// counters directly. That test passed while the production RPC was
/// answering permanent idle, because it initialised the very engine
/// singleton production had stopped booting — the assertion held against a
/// path nothing reached.
#[tokio::test]
async fn ingestion_status_reports_the_bound_drivers_queue() {
    let (_tmp, config) = bind_queue(crate::openhuman::memory::api::provider::types::QueueStats {
        ready: 3,
        running: 1,
        last_completed_ms: Some(1_700_000_000_000),
        ..Default::default()
    });

    let status = ingestion_status_for_config(&config)
        .await
        .expect("ingestion status");

    assert!(status.running, "a held job means the queue is working");
    assert_eq!(status.queue_depth, 3, "queue_depth is the ready count");
    assert_eq!(status.last_completed_at, Some(1_700_000_000_000));

    // The reduction, asserted rather than assumed: `QueueStats` is counts,
    // not job identity, so nothing fills these and nothing is invented for
    // them. If a future contract member does carry the in-flight document,
    // this is the test that should stop compiling as written.
    assert!(status.current_document_id.is_none());
    assert!(status.current_title.is_none());
    assert!(status.current_namespace.is_none());
    assert!(status.last_document_id.is_none());
    assert!(status.last_success.is_none());
}

/// An idle queue reports idle — the answer the broken handler used to give
/// unconditionally, now given only when the driver actually says so.
#[tokio::test]
async fn ingestion_status_reports_idle_for_an_empty_queue() {
    let (_tmp, config) = bind_queue(Default::default());

    let status = ingestion_status_for_config(&config)
        .await
        .expect("ingestion status");

    assert!(!status.running);
    assert_eq!(status.queue_depth, 0);
    assert!(status.last_completed_at.is_none());
}
