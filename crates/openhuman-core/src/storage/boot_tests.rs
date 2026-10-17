use super::*;
use serde_json::json;
use tinystoragedrivers::{MemoryStorage, Precondition};

#[tokio::test]
async fn an_empty_backend_prepares_and_migrates_cleanly() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let report = prepare_and_migrate(&backend).await.unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(report.scopes >= 1);
    // A second boot has nothing left to do.
    let again = prepare_and_migrate(&backend).await.unwrap();
    assert!(again.failures.is_empty(), "{:?}", again.failures);
    assert_eq!(again.steps_run, 0);
}

#[tokio::test]
async fn a_v1_agent_scope_is_moved_before_its_owner_steps_run() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let old = backend.for_scope(&Scope::new("asha").unwrap()).unwrap();
    old.documents()
        .put(
            "harness_embedding_cache",
            "legacy-id",
            json!({"sig": "s", "key": "k", "vector": [1.0]}),
            Precondition::None,
        )
        .await
        .unwrap();
    let report = prepare_and_migrate(&backend).await.unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);

    let new = backend
        .for_scope(&Scope::local().agent("asha").unwrap())
        .unwrap();
    assert!(new
        .documents()
        .get("harness.embedding_cache", "s/k")
        .await
        .unwrap()
        .is_some());
}
