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

#[test]
fn get_or_create_is_atomic_for_concurrent_first_turns() {
    let dir = tempdir().unwrap();
    let store = std::sync::Arc::new(SessionStore::open(dir.path()));
    let mut workers = Vec::new();
    for _ in 0..16 {
        let store = std::sync::Arc::clone(&store);
        workers.push(std::thread::spawn(move || {
            store.get_or_create("conversation", "conversation:prompt")
        }));
    }
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|(_, is_new)| *is_new).count(), 1);
    assert!(results.windows(2).all(|pair| pair[0].0 == pair[1].0));
}

#[test]
fn a_returning_prompt_starts_a_new_epoch() {
    let dir = tempdir().unwrap();
    let store = SessionStore::open(dir.path());
    let (first, _) = store.get_or_create("conversation", "conversation:prompt-a");
    let (_, _) = store.get_or_create("conversation", "conversation:prompt-b");
    let (returned, is_new) = store.get_or_create("conversation", "conversation:prompt-a");
    assert!(is_new);
    assert_ne!(first, returned);
}
