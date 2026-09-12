use super::*;

/// Write a cache file directly with a controlled `fetched_at_epoch`.
fn write_cache(dir: &std::path::Path, fetched_at_epoch: u64) {
    let cache = CatalogCache {
        entries: Vec::new(),
        fetched_at_epoch,
    };
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(CACHE_FILE), serde_json::to_string(&cache).unwrap()).unwrap();
}

#[test]
fn within_ttl_is_fresh_past_ttl_is_stale() {
    let _guard = crate::openhuman::skills::catalog::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var(CACHE_DIR_ENV, tmp.path());

    // Fresh: just written.
    write_cache(tmp.path(), now_epoch());
    assert!(matches!(
        load_cached_catalog_state(),
        Some(CachedCatalog::Fresh(_))
    ));
    assert!(load_cached_catalog().is_some());

    // Stale: older than the TTL. `load_cached_catalog` (TTL-respecting)
    // returns None, but the state loader still surfaces the entries.
    write_cache(tmp.path(), now_epoch().saturating_sub(CACHE_TTL_SECS + 60));
    assert!(matches!(
        load_cached_catalog_state(),
        Some(CachedCatalog::Stale(_))
    ));
    assert!(load_cached_catalog().is_none());

    std::env::remove_var(CACHE_DIR_ENV);
}

#[test]
fn missing_cache_returns_none() {
    let _guard = crate::openhuman::skills::catalog::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var(CACHE_DIR_ENV, tmp.path());
    assert!(load_cached_catalog_state().is_none());
    assert!(load_cached_catalog().is_none());
    std::env::remove_var(CACHE_DIR_ENV);
}
