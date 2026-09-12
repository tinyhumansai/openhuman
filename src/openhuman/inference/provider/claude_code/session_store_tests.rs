use super::*;
use tempfile::tempdir;

#[test]
fn uuid_v4_format() {
    let id = generate_uuid_v4();
    assert!(is_uuid_v4(&id), "generated id should be v4: {id}");
}

#[test]
fn rejects_non_v4() {
    assert!(!is_uuid_v4("not-a-uuid"));
    assert!(!is_uuid_v4("cc_abc123"));
    // version 1 uuid (nibble at 14 is '1')
    assert!(!is_uuid_v4("00000000-0000-1000-8000-000000000000"));
}

#[test]
fn roundtrip_set_and_get() {
    let dir = tempdir().unwrap();
    let store = SessionStore::open(dir.path());
    assert!(store.get("thread_a").is_none());
    store.set("thread_a", "abc").unwrap();
    let reopened = SessionStore::open(dir.path());
    assert_eq!(reopened.get("thread_a").as_deref(), Some("abc"));
}
