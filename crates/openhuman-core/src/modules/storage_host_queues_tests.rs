//! A module granted a queue manifest uses `Storage::queues()` over the
//! real bus: the host declares the manifest's queues on the module's scope,
//! refuses a queue the manifest does not name, and denies a module whose grant
//! holds no queues at all, while that module still connects.

use super::*;
use crate::modules::storage_queue_manifest::QueueManifest;
use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::Connection;
use tinystoragedrivers::ErrorKind;
use tinystoragedrivers_api::QueueStore;
use tinystoragedrivers_testkit::MemoryBackend as MemoryStorage;
use tinystoragedrivers_types::{NewMessage, QueueFeatures, QueueSpec};

fn manifest() -> Arc<QueueManifest> {
    Arc::new(
        QueueManifest::from_json(
            r#"{"version":1,"queues":[{"name":"inbox","description":"test","owner":"test",
                "max_attempts":3,"dedupe_window_ms":60000,"visibility_ms":30000}]}"#,
        )
        .unwrap(),
    )
}

struct Peers {
    queued: Connection,
    plain: Connection,
    _service: Connection,
    _served: tinystoragedrivers_busproxy::ServedStorage,
    backend: Arc<dyn StorageBackend>,
}

/// A bus where `queued` is module `alpha` (granted `inbox`) and `plain` is
/// module `beta` (documents only).
async fn peers() -> Peers {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let connect = || async {
        Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap()
    };
    let service = connect().await;
    let queued = connect().await;
    let plain = connect().await;

    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let mut alpha = grant("notes");
    alpha.queues = Some(manifest());
    let env = Arc::new(Env {
        backend: Mutex::new(Some(Arc::clone(&backend))),
        profile: Mutex::new(None),
        grants: HashMap::from([("alpha", alpha), ("beta", grant("other"))]),
        fences: Mutex::new(None),
        keys_ready: AtomicBool::new(true),
    });
    let directory = Directory(HashMap::from([
        (
            queued.unique_name().unwrap().as_str().to_string(),
            "alpha".to_string(),
        ),
        (
            plain.unique_name().unwrap().as_str().to_string(),
            "beta".to_string(),
        ),
    ]));
    let host = Arc::new(StorageHost::new(
        Arc::new(directory),
        env as Arc<dyn Environment>,
        Arc::new(Trail::default()) as Arc<dyn AuditSink>,
    ));
    let served = serve_objects(&service, &host, None).await.unwrap();
    Peers {
        queued,
        plain,
        _service: service,
        _served: served,
        backend,
    }
}

async fn queues_of(connection: &Connection, _module_id: &str) -> Arc<dyn QueueStore> {
    let storage = tinystoragedrivers_busproxy::connect_storage(connection.clone())
        .await
        .expect("every module connects, with or without queues");
    Arc::clone(storage.queues())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_module_granted_a_manifest_enqueues_receives_and_acks_over_the_bus() {
    let peers = peers().await;
    let queues = queues_of(&peers.queued, "alpha").await;

    // Agreeing with the declared spec is accepted; the host already declared it.
    let declared = manifest().get("inbox").unwrap().spec();
    queues.ensure_queue("inbox", &declared).await.unwrap();

    let enqueued = queues
        .enqueue("inbox", &[NewMessage::new(json!({"n": 1}))])
        .await
        .unwrap();
    assert_eq!(enqueued.len(), 1);
    let leased = queues.receive("inbox", 4, 30_000, "test").await.unwrap();
    assert_eq!(leased.len(), 1);
    assert_eq!(leased[0].payload, json!({"n": 1}));
    assert!(queues.ack("inbox", &leased[0].receipt).await.unwrap());
    assert!(queues
        .receive("inbox", 4, 30_000, "test")
        .await
        .unwrap()
        .is_empty());

    // The messages live in the module's own scope on the host backend.
    let scope = module_scope(None, "alpha").unwrap();
    let host_side = peers.backend.for_scope(&scope).unwrap();
    let stats = host_side.queues().stats("inbox").await.unwrap();
    assert_eq!(stats.ready, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queue_outside_the_manifest_is_not_found_and_a_divergent_spec_conflicts() {
    let peers = peers().await;
    let queues = queues_of(&peers.queued, "alpha").await;

    let error = queues
        .enqueue("elsewhere", &[NewMessage::new(json!({}))])
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");

    let mut divergent: QueueSpec = manifest().get("inbox").unwrap().spec();
    divergent.max_attempts += 1;
    let error = queues.ensure_queue("inbox", &divergent).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Conflict, "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_module_without_a_queue_grant_connects_but_is_denied_queues() {
    let peers = peers().await;
    let queues = queues_of(&peers.plain, "beta").await;
    assert_eq!(queues.features(), QueueFeatures::default());
    let error = queues
        .enqueue("inbox", &[NewMessage::new(json!({}))])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("object-not-granted"), "{error}");
}
