use super::*;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use serde_json::json;
use tinyflows_drivers::catalog::FlowCatalogDocuments;

fn documents(namespace: &str) -> FlowState {
    documents_on(&MemoryStorage::new(), namespace)
}

fn documents_on(storage: &MemoryStorage, namespace: &str) -> FlowState {
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    FlowState::Documents(FlowStateDocuments::new(
        FlowCatalogDocuments::new(Arc::clone(scoped.documents())),
        namespace,
    ))
}

/// What a run stores through `StateStore` is what dedup settlement reads
/// through `DedupKv`, on both stores.
async fn engine_and_dedup_share_records(state: FlowState) {
    state.store("seen", json!(["a"])).await.unwrap();
    assert_eq!(state.kv_get("seen").unwrap(), Some(json!(["a"])));
    state.kv_set("seen", &json!(["a", "b"])).unwrap();
    assert_eq!(state.load("seen").await.unwrap(), Some(json!(["a", "b"])));
    state.kv_delete("seen").unwrap();
    assert_eq!(state.load("seen").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_document_store_backs_both_views() {
    engine_and_dedup_share_records(documents("flow:a")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_database_backs_both_views() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    engine_and_dedup_share_records(FlowState::open(&config, "flow:a")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn namespaces_keep_flows_apart() {
    let storage = MemoryStorage::new();
    let (a, b) = (
        documents_on(&storage, "flow:a"),
        documents_on(&storage, "flow:b"),
    );
    a.store("k", json!(1)).await.unwrap();
    assert_eq!(b.load("k").await.unwrap(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn an_unresolved_scope_fails_every_call() {
    let state = FlowState::Unavailable("no acting agent".to_string());
    assert!(state.load("k").await.is_err());
    assert!(state.store("k", json!(1)).await.is_err());
    let error = state.kv_get("k").unwrap_err();
    assert!(error.contains("no acting agent"), "{error}");
    assert!(state.kv_set("k", &json!(1)).is_err());
    assert!(state.kv_delete("k").is_err());
}

#[test]
fn without_a_backend_the_flow_state_is_in_the_workspace_default_database() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    // Nothing in this binary installs a backend, so the slot is empty.
    assert!(crate::storage::installed().is_none());
    assert!(matches!(
        FlowState::open(&config, "flow:a"),
        FlowState::Documents(_)
    ));
    assert!(config.workspace_dir.join("storage/storage.db").is_file());
}
