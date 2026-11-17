use tiny_artifacts::{ArtifactKind, ArtifactRecord, ArtifactStore};

use super::*;

fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("version-docs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn record(id: &str, body: &str, at: u64) -> ArtifactRecord {
    ArtifactRecord::new(id, "Notes", ArtifactKind::Markdown, body, "agent", at)
}

#[tokio::test]
async fn records_round_trip_through_the_document_store() {
    let ws = workspace();
    let store = DocumentArtifactStore::new(&ws);
    store.upsert(&record("a-1", "# one", 1)).await.unwrap();
    store.upsert(&record("a-2", "# two", 2)).await.unwrap();

    let got = store.get("a-1").await.unwrap().unwrap();
    assert_eq!(got, record("a-1", "# one", 1));
    let listed: Vec<_> = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    // Most recently updated first.
    assert_eq!(listed, vec!["a-2", "a-1"]);

    assert!(store.delete("a-1").await.unwrap());
    assert!(store.get("a-1").await.unwrap().is_none());
    assert!(!store.delete("a-1").await.unwrap());
    // Nothing is written as a file.
    assert!(!super::super::tiny_store::legacy_versioned_dir(&ws)
        .join("a-2.json")
        .exists());
}

#[tokio::test]
async fn records_from_an_earlier_build_are_imported_once_and_never_resurrected() {
    let ws = workspace();
    let legacy = super::super::tiny_store::legacy_versioned_dir(&ws);
    std::fs::create_dir_all(&legacy).unwrap();
    let old = record("old-1", "# kept as a file", 5);
    std::fs::write(legacy.join("old-1.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    // A copy under another name is not the record; junk is skipped.
    std::fs::write(legacy.join("other.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    std::fs::write(legacy.join("junk.json"), b"not json").unwrap();

    let store = DocumentArtifactStore::new(&ws);
    assert_eq!(store.get("old-1").await.unwrap(), Some(old.clone()));
    assert!(store.get("other").await.unwrap().is_none());

    // A newer write is not overwritten by a repeated import.
    store.upsert(&record("old-1", "# newer", 9)).await.unwrap();
    assert_eq!(
        import_legacy(
            &Repo::current_for(&ws, DOMAIN, collections).unwrap(),
            &legacy
        )
        .unwrap(),
        0
    );
    assert_eq!(
        store
            .get("old-1")
            .await
            .unwrap()
            .unwrap()
            .latest()
            .unwrap()
            .body,
        "# newer"
    );

    // Deleting removes the legacy file too, so the next import cannot bring it back.
    assert!(store.delete("old-1").await.unwrap());
    assert!(!legacy.join("old-1.json").exists());
    import_legacy(
        &Repo::current_for(&ws, DOMAIN, collections).unwrap(),
        &legacy,
    )
    .unwrap();
    assert!(store.get("old-1").await.unwrap().is_none());
}
