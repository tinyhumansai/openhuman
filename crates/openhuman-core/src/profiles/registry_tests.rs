use super::*;
use crate::storage::test_support::memory_backend;
#[cfg(feature = "storage-sqlite")]
use crate::storage::test_support::sqlite_backend;
#[cfg(feature = "storage-sqlite")]
use tinystoragedrivers_types::Precondition;
use tinystoragedrivers_types::{CollectionSpec, Scope};

fn meta(id: &str, created_at: u64) -> ProfileMeta {
    ProfileMeta {
        profile_id: ProfileId::parse(id).unwrap(),
        created_at,
        layout_version: super::super::types::LAYOUT_VERSION,
    }
}

async fn exercise(registry: &ProfileRegistry) {
    let alice = meta("alice", 1);
    assert!(registry.get(&alice.profile_id).await.unwrap().is_none());
    assert!(registry.create(&alice).await.unwrap());
    assert!(
        !registry.create(&meta("alice", 2)).await.unwrap(),
        "a second create keeps the first record"
    );
    assert_eq!(registry.get(&alice.profile_id).await.unwrap(), Some(alice));
    assert!(registry.create(&meta("bob", 3)).await.unwrap());
    let ids: Vec<_> = registry
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.profile_id.to_string())
        .collect();
    assert_eq!(ids, vec!["alice", "bob"]);
}

#[tokio::test]
async fn the_shared_registry_records_lists_and_forgets() {
    let storage = memory_backend();
    let registry = ProfileRegistry::cluster(storage.as_ref()).unwrap();
    assert!(registry.is_shared());
    exercise(&registry).await;

    // A second node over the same backend sees the same profiles.
    let other = ProfileRegistry::cluster(storage.as_ref()).unwrap();
    assert_eq!(other.list().await.unwrap().len(), 2);

    let alice = ProfileId::parse("alice").unwrap();
    assert!(registry.remove(&alice).await.unwrap());
    assert!(!registry.remove(&alice).await.unwrap());
    assert!(other.get(&alice).await.unwrap().is_none());
}

#[tokio::test]
async fn the_file_registry_keeps_profile_toml_beside_each_profile() {
    let tmp = tempfile::tempdir().unwrap();
    for id in ["alice", "bob"] {
        let layout = ProfileLayout::new(tmp.path(), ProfileId::parse(id).unwrap());
        std::fs::create_dir_all(&layout.dir).unwrap();
    }
    let registry = ProfileRegistry::files(tmp.path());
    assert!(!registry.is_shared());
    exercise(&registry).await;
    let alice = ProfileId::parse("alice").unwrap();
    assert!(ProfileLayout::new(tmp.path(), &alice).meta_path.is_file());

    // An unreadable record is skipped, not fatal; a stray directory is not
    // a profile.
    std::fs::write(ProfileLayout::new(tmp.path(), &alice).meta_path, "nope = [").unwrap();
    std::fs::create_dir_all(layout::users_dir(tmp.path()).join("Not An Id")).unwrap();
    let listed = registry.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].profile_id.as_str(), "bob");
}

#[tokio::test]
async fn an_empty_root_lists_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(ProfileRegistry::files(tmp.path().join("missing"))
        .list()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn doctor_lookup_of_an_unknown_profile_does_not_declare_cluster_schema() {
    let storage = memory_backend();
    let registry = ProfileRegistry::cluster(storage.as_ref()).unwrap();
    let docs = storage
        .for_scope(&Scope::new(crate::storage::lease::CLUSTER_SCOPE).unwrap())
        .unwrap();
    let registry_spec = CollectionSpec::new(REGISTRY_COLLECTION);
    let lease_spec = CollectionSpec::new(crate::storage::lease::LEASE_COLLECTION);

    assert!(registry
        .get_readonly(&ProfileId::parse("alice").unwrap())
        .await
        .unwrap()
        .is_none());

    assert!(docs
        .documents()
        .validate_collection_spec(&registry_spec)
        .await
        .is_err());
    assert!(docs
        .documents()
        .validate_collection_spec(&lease_spec)
        .await
        .is_err());
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn doctor_lookup_does_not_migrate_legacy_profile_or_lease_records() {
    let storage = sqlite_backend().unwrap();
    let registry = ProfileRegistry::cluster(storage.backend.as_ref()).unwrap();
    let scoped = storage
        .backend
        .for_scope(&Scope::new(crate::storage::lease::CLUSTER_SCOPE).unwrap())
        .unwrap();
    let docs = scoped.documents();
    let alice_key = crate::storage::schema_support::key_of("alice").unwrap();
    docs.ensure_collection(&CollectionSpec::new("profiles"))
        .await
        .unwrap();
    docs.put(
        "profiles",
        &alice_key,
        serde_json::json!({ "profile_id": "alice", "created_at": 1, "layout_version": 2 }),
        Precondition::Absent,
    )
    .await
    .unwrap();
    docs.ensure_collection(&CollectionSpec::new("leases"))
        .await
        .unwrap();
    docs.put(
        "leases",
        "alice",
        serde_json::json!({
            "owner": "node-a",
            "endpoint": null,
            "epoch": 3,
            "expires_at_ms": u64::MAX,
            "released": false
        }),
        Precondition::Absent,
    )
    .await
    .unwrap();

    let result = registry
        .get_readonly(&ProfileId::parse("alice").unwrap())
        .await;
    assert!(
        result.is_err() || result.unwrap().is_some(),
        "a legacy provisioned profile must not be misreported as unprovisioned"
    );

    assert!(docs.get("profiles", &alice_key).await.unwrap().is_some());
    assert!(docs.get("leases", "alice").await.unwrap().is_some());
    assert!(docs
        .validate_collection_spec(&CollectionSpec::new(REGISTRY_COLLECTION))
        .await
        .is_ok());
    assert!(docs
        .validate_collection_spec(&CollectionSpec::new(
            crate::storage::lease::LEASE_COLLECTION
        ))
        .await
        .is_err());
}
