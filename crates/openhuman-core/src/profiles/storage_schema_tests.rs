use super::*;
#[cfg(feature = "storage-sqlite")]
use crate::storage::test_support::sqlite_backend;
#[cfg(feature = "storage-sqlite")]
use crate::storage::Scope;
use serde_json::json;
#[cfg(feature = "storage-sqlite")]
use std::sync::Arc;
use tinystoragedrivers::Meta;
#[cfg(feature = "storage-sqlite")]
use tinystoragedrivers::Precondition;

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
#[cfg(feature = "storage-sqlite")]
async fn cluster_schema_declaration_leaves_legacy_registry_for_the_host_importer() {
    let fixture = sqlite_backend().unwrap();
    let backend = Arc::clone(&fixture.backend);
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
    assert!(docs
        .get(REGISTRY_COLLECTION, "alice")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        docs.get("profiles", "alice").await.unwrap().unwrap().doc,
        json!({ "profile_id": "alice", "created_at": 5, "layout_version": 2 })
    );
    assert!(docs
        .validate_collection_spec(&CollectionSpec::new(REGISTRY_COLLECTION))
        .await
        .is_ok());
}

#[test]
fn the_schema_composes_under_its_owner() {
    assert_eq!(
        collection_specs(),
        vec![CollectionSpec::new(REGISTRY_COLLECTION)]
    );
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn legacy_lease_conversion_preserves_epoch_and_source_records() {
    use crate::storage::lease::LEASE_OWNER;
    let fixture = sqlite_backend().unwrap();
    let backend = Arc::clone(&fixture.backend);
    let scoped = backend.for_scope(&Scope::cluster()).unwrap();
    let docs = scoped.documents();
    docs.ensure_collection(&CollectionSpec::new("leases"))
        .await
        .unwrap();
    docs.put(
        "leases",
        "alice",
        json!({ "owner": "node-a", "endpoint": "http://a", "epoch": 4,
                "expires_at_ms": 5_000, "released": false }),
        Precondition::Absent,
    )
    .await
    .unwrap();
    docs.put(
        "leases",
        "bob",
        json!({ "owner": "node-a", "endpoint": null, "epoch": 2,
                "expires_at_ms": 1_000, "released": true }),
        Precondition::Absent,
    )
    .await
    .unwrap();
    let converted = upgrade_lease_records(vec![
        crate::storage::import::Record {
            collection: "leases".into(),
            id: "alice".into(),
            doc: json!({ "owner": "node-a", "endpoint": "http://a", "epoch": 4,
                "expires_at_ms": 5_000, "released": false }),
            created_ms: None,
            schema: None,
            expires_ms: None,
            updated_ms: None,
        },
        crate::storage::import::Record {
            collection: "leases".into(),
            id: "bob".into(),
            doc: json!({ "owner": "node-a", "endpoint": null, "epoch": 2,
                "expires_at_ms": 1_000, "released": true }),
            created_ms: None,
            schema: None,
            expires_ms: None,
            updated_ms: None,
        },
    ])
    .unwrap();
    assert_eq!(converted.len(), 2);
    assert_eq!(converted[0].collection, "openhuman.leases");
    assert_eq!(
        converted[0].id,
        crate::storage::schema_support::key_of_parts([LEASE_OWNER, "alice"]).unwrap()
    );
    let moved = &converted[0];
    assert_eq!(
        moved.doc,
        json!({ "holder": "node-a", "epoch": 4, "expires_ms": 5_000, "endpoint": "http://a" })
    );
    assert_eq!(converted[1].doc["released"], true);
    assert!(docs.get("leases", "alice").await.unwrap().is_some());
    assert!(docs
        .get("tsd.leases", &converted[0].id)
        .await
        .unwrap()
        .is_none());
}
