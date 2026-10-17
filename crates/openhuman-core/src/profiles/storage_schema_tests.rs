use super::*;
use serde_json::json;
use tinystoragedrivers::{MemoryStorage, Meta, Precondition, Scope, StorageBackend};

#[test]
fn a_registry_row_loses_its_seconds_field_and_keeps_its_creation_time() {
    let out = registry_v2(vec![SourceRecord {
        id: "alice".into(),
        body: json!({ "profile_id": "alice", "created_at": 1_700_000_000_u64, "layout_version": 2 }),
        meta: Meta::unknown(),
    }])
    .unwrap();
    assert_eq!(out[0].collection, REGISTRY_COLLECTION);
    assert_eq!(out[0].id, "alice");
    assert_eq!(
        out[0].body,
        json!({ "profile_id": "alice", "layout_version": 2 })
    );
    assert_eq!(out[0].created_ms, Some(1_700_000_000_000));
}

#[tokio::test]
async fn migrate_cluster_moves_the_v1_registry_into_the_v2_collection() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::cluster()).unwrap();
    let docs = scoped.documents();
    docs.ensure_collection(&CollectionSpec::new("profiles"))
        .await
        .unwrap();
    docs.put(
        "profiles",
        "alice",
        json!({ "profile_id": "alice", "created_at": 5, "layout_version": 2 }),
        Precondition::Absent,
    )
    .await
    .unwrap();
    migrate_cluster(&scoped).await.unwrap();
    migrate_cluster(&scoped).await.unwrap();
    let moved = docs
        .get(REGISTRY_COLLECTION, "alice")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        moved.doc,
        json!({ "profile_id": "alice", "layout_version": 2 })
    );
    assert_eq!(moved.meta.created_ms, 5000);
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
