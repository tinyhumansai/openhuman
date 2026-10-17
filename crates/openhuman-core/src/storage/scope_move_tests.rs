use super::*;
use serde_json::json;
use tinystoragedrivers::MemoryStorage;

fn known() -> Known {
    Known {
        agents: vec!["has a space".into(), "asha".into()],
        profiles: vec!["has a space".into()],
        modules: vec!["tinymcp".into()],
    }
}

fn mapped(src: &str) -> String {
    match map_scope(src, &known()) {
        Mapping::Move(scope) => scope.as_str().to_owned(),
        other => format!("{other:?}"),
    }
}

#[test]
fn every_v1_shape_maps_to_a_v2_scope() {
    assert_eq!(mapped("asha"), "local/agent:asha");
    assert_eq!(mapped("default"), "local/agent:default");
    assert_eq!(mapped("profile:alice"), "u:alice");
    assert_eq!(mapped("alice~asha"), "u:alice/agent:asha");
    assert_eq!(mapped("alice~default"), "u:alice/agent:default");
    assert_eq!(
        mapped("a%7Eb~asha"),
        format!("{}/agent:asha", scope_for_profile("a~b"))
    );
    assert_eq!(mapped("alice~module:tinymcp"), "u:alice/module:tinymcp");
    assert_eq!(mapped("module:tinymcp@local"), "local/module:tinymcp");
    assert_eq!(mapped("module:tinymcp@asha"), "local/module:tinymcp");
}

#[test]
fn hashed_v1_scopes_map_through_known_ids_only() {
    let agent = format!("sha256:{}", sha_hex(b"has a space"));
    assert_eq!(mapped(&agent), scope_for_agent("has a space").as_str());
    let profile = format!("profile-sha256:{}", sha_hex(b"has a space"));
    assert_eq!(mapped(&profile), scope_for_profile("has a space").as_str());
    assert_eq!(mapped(&format!("sha256:{}", sha_hex(b"nobody"))), "Unknown");
}

#[test]
fn v2_scopes_are_left_alone() {
    for scope in [
        "local",
        "u:alice",
        "u:alice/agent:x",
        "local/module:m",
        "cluster",
    ] {
        assert_eq!(map_scope(scope, &known()), Mapping::V2, "{scope}");
    }
}

#[tokio::test]
async fn a_v1_scope_moves_its_documents_streams_and_blobs() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let old = backend
        .for_scope(&Scope::new("alice~asha").unwrap())
        .unwrap();
    old.documents()
        .put("notes", "n1", json!({"a": 1}), Precondition::None)
        .await
        .unwrap();
    old.streams()
        .append("journal", json!({"k": 1}))
        .await
        .unwrap();
    old.streams()
        .append("journal", json!({"k": 2}))
        .await
        .unwrap();
    old.blobs()
        .put("b/1", b"hi".to_vec(), Some("text/plain"))
        .await
        .unwrap();

    let report = run(&backend).await.unwrap();
    assert_eq!(report.moved, 1);
    assert!(report.documents >= 1);
    assert_eq!(report.entries, 2);
    assert_eq!(report.blobs, 1);

    let new = backend
        .for_scope(&scope_for_profile("alice").agent("asha").unwrap())
        .unwrap();
    assert_eq!(
        new.documents()
            .get("notes", "n1")
            .await
            .unwrap()
            .unwrap()
            .doc,
        json!({"a": 1})
    );
    assert_eq!(new.streams().len("journal").await.unwrap(), 2);
    assert_eq!(new.blobs().get("b/1").await.unwrap().unwrap().bytes, b"hi");

    // The v1 scope is gone and a second run is a no-op.
    let old = backend
        .for_scope(&Scope::new("alice~asha").unwrap())
        .unwrap();
    assert!(old.documents().get("notes", "n1").await.unwrap().is_none());
    assert_eq!(old.streams().len("journal").await.unwrap(), 0);
    let again = run(&backend).await.unwrap();
    assert_eq!(again.moved, 0);
}

#[tokio::test]
async fn a_record_already_in_the_target_wins_a_merge() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let target = backend.for_scope(&scope_for_agent("asha")).unwrap();
    target
        .documents()
        .put("notes", "n", json!({"from": "v2"}), Precondition::None)
        .await
        .unwrap();
    let old = backend.for_scope(&Scope::new("asha").unwrap()).unwrap();
    old.documents()
        .put("notes", "n", json!({"from": "v1"}), Precondition::None)
        .await
        .unwrap();
    run(&backend).await.unwrap();
    assert_eq!(
        target
            .documents()
            .get("notes", "n")
            .await
            .unwrap()
            .unwrap()
            .doc,
        json!({"from": "v2"})
    );
}
