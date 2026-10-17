use super::*;
use tinyskills::Validators;

fn catalog(fetched_at: u64) -> StoredCatalog {
    StoredCatalog::new(Vec::new(), fetched_at, Validators::default(), 0)
}

/// Writes `<dir>/<registry>.json` the way tinyskills' removed
/// `FileCatalogStore` did: the serialized [`StoredCatalog`].
fn write_legacy(dir: &Path, registry: &str, catalog: &StoredCatalog) {
    std::fs::write(
        dir.join(format!("{registry}.json")),
        serde_json::to_vec(catalog).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn a_saved_catalog_is_read_back_without_writing_a_catalog_file() {
    let tmp = tempfile::tempdir().unwrap();
    let store = HostCatalogStore::new(tmp.path());

    assert!(store.load("hermes").await.unwrap().is_none());
    store.save("hermes", &catalog(7)).await.unwrap();

    let loaded = store.load("hermes").await.unwrap().unwrap();
    assert_eq!(loaded.fetched_at, 7);
    assert!(
        !tmp.path().join("hermes.json").exists(),
        "the catalog lives in the storage backend, not in a file"
    );
}

#[tokio::test]
async fn a_legacy_catalog_file_is_imported_once_and_left_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    write_legacy(tmp.path(), "hermes", &catalog(11));
    let legacy = tmp.path().join("hermes.json");
    let before = std::fs::read(&legacy).unwrap();
    let store = HostCatalogStore::new(tmp.path());

    assert_eq!(store.load("hermes").await.unwrap().unwrap().fetched_at, 11);
    assert_eq!(std::fs::read(&legacy).unwrap(), before, "never modified");

    // A refresh replaces the document; the older file is not read again.
    store.save("hermes", &catalog(99)).await.unwrap();
    assert_eq!(store.load("hermes").await.unwrap().unwrap().fetched_at, 99);
    assert_eq!(std::fs::read(&legacy).unwrap(), before);
}

#[tokio::test]
async fn an_unusable_legacy_file_reads_as_an_empty_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let store = HostCatalogStore::new(tmp.path());

    std::fs::write(tmp.path().join("hermes.json"), b"{ not json").unwrap();
    assert!(store.load("hermes").await.unwrap().is_none());

    std::fs::write(
        tmp.path().join("other.json"),
        serde_json::json!({ "format": 999, "entries": [] }).to_string(),
    )
    .unwrap();
    assert!(store.load("other").await.unwrap().is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_legacy_file_is_not_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    write_legacy(elsewhere.path(), "hermes", &catalog(5));
    std::os::unix::fs::symlink(
        elsewhere.path().join("hermes.json"),
        tmp.path().join("hermes.json"),
    )
    .unwrap();

    let store = HostCatalogStore::new(tmp.path());
    assert!(store.load("hermes").await.unwrap().is_none());
}

#[tokio::test]
async fn a_registry_id_that_is_not_a_path_segment_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let store = HostCatalogStore::new(tmp.path());

    assert!(matches!(
        store.load("../escape").await,
        Err(StoreError::InvalidId(_))
    ));
    assert!(matches!(
        store.save("../escape", &catalog(1)).await,
        Err(StoreError::InvalidId(_))
    ));
}

#[tokio::test]
async fn catalogs_of_different_cache_directories_do_not_mix() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    HostCatalogStore::new(a.path())
        .save("hermes", &catalog(1))
        .await
        .unwrap();

    assert!(HostCatalogStore::new(b.path())
        .load("hermes")
        .await
        .unwrap()
        .is_none());
}
