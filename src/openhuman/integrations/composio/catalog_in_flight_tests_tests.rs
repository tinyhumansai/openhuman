use super::*;

#[test]
fn catalog_miss_locks_are_shared_per_toolkit_only() {
    let first = live_catalog_fetch_lock("test-lock-a").unwrap();
    let same = live_catalog_fetch_lock("test-lock-a").unwrap();
    let other = live_catalog_fetch_lock("test-lock-b").unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &same));
    assert!(!std::sync::Arc::ptr_eq(&first, &other));
}
