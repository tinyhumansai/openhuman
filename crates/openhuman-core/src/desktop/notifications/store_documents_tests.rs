use super::*;
use crate::desktop::notifications::types::CoreNotificationCategory;
use crate::storage::{MemoryStorage, Scope, StorageBackend};

const WS: &str = "/workspace/a";

fn docs_in(storage: &MemoryStorage, scope: &str) -> Docs {
    Docs::over(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn docs() -> Docs {
    docs_in(&MemoryStorage::new(), "local")
}

fn note(id: &str, provider: &str, body: &str, secs_ago: i64) -> IntegrationNotification {
    IntegrationNotification {
        id: id.to_string(),
        provider: provider.to_string(),
        account_id: Some("acct".to_string()),
        title: "New message".to_string(),
        body: body.to_string(),
        raw_payload: json!({ "k": 1 }),
        importance_score: None,
        triage_action: None,
        triage_reason: None,
        status: NotificationStatus::Unread,
        received_at: Utc::now() - Duration::seconds(secs_ago),
        scored_at: None,
    }
}

fn event(id: &str, ts: u64) -> CoreNotificationEvent {
    CoreNotificationEvent {
        id: id.to_string(),
        category: CoreNotificationCategory::Agents,
        title: "Cron job completed".to_string(),
        body: "done".to_string(),
        deep_link: None,
        timestamp_ms: ts,
        actions: None,
        workspace: None,
        workspace_revision: None,
    }
}

fn ids(list: &[IntegrationNotification]) -> Vec<&str> {
    list.iter().map(|n| n.id.as_str()).collect()
}

#[test]
fn notifications_round_trip_newest_first() {
    let store = docs();
    assert!(store.insert(&note("old", "slack", "a", 30), false).unwrap());
    assert!(store.insert(&note("new", "gmail", "b", 0), false).unwrap());
    let listed = store.list(10, 0, None, None).unwrap();
    assert_eq!(ids(&listed), ["new", "old"]);
    assert_eq!(listed[1].raw_payload, json!({ "k": 1 }));
    assert_eq!(listed[1].account_id.as_deref(), Some("acct"));
    assert!(listed[1].importance_score.is_none());
    assert!(
        store.insert(&note("old", "slack", "z", 0), false).is_err(),
        "an id is inserted once"
    );
    assert_eq!(ids(&store.list(1, 1, None, None).unwrap()), ["old"]);
    assert!(store.list(0, 0, None, None).unwrap().is_empty());
    assert_eq!(
        ids(&store.list(10, 0, Some("slack"), None).unwrap()),
        ["old"]
    );
}

#[test]
fn identical_content_within_a_minute_is_skipped() {
    let store = docs();
    assert!(store.insert(&note("a", "slack", "hi", 0), true).unwrap());
    assert!(!store.insert(&note("b", "slack", "hi", 0), true).unwrap());
    assert!(store.insert(&note("c", "slack", "other", 0), true).unwrap());
    assert!(store
        .exists_recent("slack", Some("acct"), "New message", "hi")
        .unwrap());
    assert!(!store
        .exists_recent("slack", None, "New message", "hi")
        .unwrap());
    assert!(!store
        .exists_recent("gmail", Some("acct"), "New message", "hi")
        .unwrap());
    assert_eq!(ids(&store.list(10, 0, None, None).unwrap()).len(), 2);
}

#[test]
fn content_older_than_the_window_is_not_a_duplicate() {
    let store = docs();
    let dedup = dedup_id("slack", Some("acct"), "New message", "hi");
    let stale = (Utc::now() - Duration::seconds(120)).timestamp_millis();
    store
        .0
        .run(|docs| async move {
            docs.put(
                DEDUP,
                &dedup,
                json!({ "last_ms": stale }),
                Precondition::Absent,
            )
            .await
            .map(|_| ())
        })
        .unwrap();
    assert!(!store
        .exists_recent("slack", Some("acct"), "New message", "hi")
        .unwrap());
    assert!(store.insert(&note("b", "slack", "hi", 0), true).unwrap());
}

#[test]
fn dedup_follows_arrival_not_the_notifications_own_timestamp() {
    let store = docs();
    // A delayed event (stamped two minutes ago) and a future-dated one both
    // arrived now, so identical content right after is a duplicate either way.
    assert!(store
        .insert(&note("a", "slack", "late", 120), true)
        .unwrap());
    assert!(!store.insert(&note("b", "slack", "late", 0), true).unwrap());
    assert!(store
        .insert(&note("c", "slack", "future", -3600), true)
        .unwrap());
    assert!(!store
        .insert(&note("d", "slack", "future", 0), true)
        .unwrap());
}

#[test]
fn a_failed_insert_releases_its_dedup_claim() {
    let store = docs();
    store
        .insert(&note("taken", "slack", "first", 0), false)
        .unwrap();
    // Same id, new content: the claim succeeds, the notification put fails.
    assert!(store
        .insert(&note("taken", "slack", "second", 0), true)
        .is_err());
    assert!(
        !store
            .exists_recent("slack", Some("acct"), "New message", "second")
            .unwrap(),
        "the claim was rolled back"
    );
    assert!(store
        .insert(&note("fresh", "slack", "second", 0), true)
        .unwrap());
}

#[test]
fn workspaces_keep_core_notifications_apart() {
    let store = docs();
    assert!(store
        .insert_core_notification("/workspace/a", &event("e", 1))
        .unwrap());
    assert!(store
        .insert_core_notification("/workspace/b", &event("e", 2))
        .unwrap());
    let a = store
        .list_core_notifications("/workspace/a", false, 10)
        .unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].timestamp_ms, 1);
    assert!(store
        .mark_core_notification_read("/workspace/a", "e")
        .unwrap());
    assert_eq!(
        store
            .unread_core_notification_count("/workspace/a")
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .unread_core_notification_count("/workspace/b")
            .unwrap(),
        1
    );
    assert!(!store
        .mark_core_notification_read("/workspace/c", "e")
        .unwrap());
}

#[test]
fn concurrent_duplicates_insert_once() {
    let store = docs();
    let inserted: usize = (0..8)
        .map(|i| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .insert(&note(&format!("n{i}"), "slack", "same", 0), true)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| usize::from(handle.join().unwrap()))
        .sum();
    assert_eq!(inserted, 1);
}

#[test]
fn triage_status_and_score_filter() {
    let store = docs();
    store.insert(&note("low", "slack", "a", 2), false).unwrap();
    store.insert(&note("high", "slack", "b", 1), false).unwrap();
    store
        .insert(&note("unscored", "slack", "c", 0), false)
        .unwrap();
    assert!(store.update_triage("low", 0.2, "drop", "noise").unwrap());
    assert!(store.update_triage("high", 0.9, "react", "urgent").unwrap());
    assert!(!store.update_triage("missing", 0.5, "drop", "x").unwrap());
    let filtered = store.list(10, 0, None, Some(0.5)).unwrap();
    assert_eq!(ids(&filtered), ["unscored", "high"]);
    assert_eq!(filtered[1].triage_action.as_deref(), Some("react"));
    assert!(filtered[1].scored_at.is_some());

    assert_eq!(store.unread_count().unwrap(), 3);
    assert!(store.set_status("low", NotificationStatus::Read).unwrap());
    assert!(store.set_status("high", NotificationStatus::Acted).unwrap());
    assert!(!store
        .set_status("missing", NotificationStatus::Dismissed)
        .unwrap());
    assert_eq!(store.unread_count().unwrap(), 1);

    let stats = store.stats().unwrap();
    assert_eq!((stats.total, stats.unread, stats.unscored), (3, 1, 1));
    assert_eq!(stats.by_provider.get("slack"), Some(&3));
    assert_eq!(stats.by_action.get("drop"), Some(&1));
    assert_eq!(stats.by_action.get("react"), Some(&1));
}

#[test]
fn settings_default_then_upsert() {
    let store = docs();
    let defaults = store.get_settings("slack").unwrap();
    assert_eq!(defaults.provider, "slack");
    let mut settings = defaults.clone();
    settings.enabled = !defaults.enabled;
    settings.importance_threshold = 0.25;
    settings.route_to_orchestrator = !defaults.route_to_orchestrator;
    store.upsert_settings(&settings).unwrap();
    let read = store.get_settings("slack").unwrap();
    assert_eq!(read.enabled, settings.enabled);
    assert_eq!(read.importance_threshold, 0.25);
    assert_eq!(read.route_to_orchestrator, settings.route_to_orchestrator);
    assert_eq!(
        store.get_settings("gmail").unwrap().enabled,
        defaults.enabled
    );
}

#[test]
fn core_notifications_persist_once_and_mark_read() {
    let store = docs();
    assert!(store.insert_core_notification(WS, &event("a", 1)).unwrap());
    assert!(!store.insert_core_notification(WS, &event("a", 1)).unwrap());
    assert!(store.insert_core_notification(WS, &event("b", 2)).unwrap());
    let all = store.list_core_notifications(WS, false, 10).unwrap();
    assert_eq!(
        all.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(all[0], event("b", 2));
    assert_eq!(store.unread_core_notification_count(WS).unwrap(), 2);
    assert!(store.mark_core_notification_read(WS, "b").unwrap());
    assert!(
        store.mark_core_notification_read(WS, "b").unwrap(),
        "still exists"
    );
    assert!(!store.mark_core_notification_read(WS, "missing").unwrap());
    let unread = store.list_core_notifications(WS, true, 10).unwrap();
    assert_eq!(
        unread.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(store.unread_core_notification_count(WS).unwrap(), 1);
    assert!(store
        .list_core_notifications(WS, false, 0)
        .unwrap()
        .is_empty());
}

#[test]
fn a_corrupt_core_payload_is_skipped() {
    let store = docs();
    store
        .insert_core_notification(WS, &event("good", 1))
        .unwrap();
    store
        .0
        .run(|docs| async move {
            docs.put(
                CORE,
                "bad",
                json!({ "workspace": WS, "payload": "not json", "timestamp_ms": 2, "read": false }),
                Precondition::Absent,
            )
            .await
            .map(|_| ())
        })
        .unwrap();
    let listed = store.list_core_notifications(WS, false, 10).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "good");
}

#[test]
fn an_unparseable_raw_payload_reads_as_a_string() {
    let stored = Versioned {
        id: "x".to_string(),
        version: tinystoragedrivers::Version::FIRST,
        doc: json!({ "raw_payload": "plain text", "status": "weird" }),
        meta: tinystoragedrivers::Meta::unknown(),
    };
    let read = to_notification(&stored);
    assert_eq!(read.raw_payload, json!("plain text"));
    assert_eq!(read.status, NotificationStatus::Unread);
}

#[test]
fn scopes_keep_notifications_apart() {
    let storage = MemoryStorage::new();
    docs_in(&storage, "alice")
        .insert(&note("a", "slack", "hi", 0), true)
        .unwrap();
    let bob = docs_in(&storage, "bob");
    assert!(bob.list(10, 0, None, None).unwrap().is_empty());
    assert!(!bob
        .exists_recent("slack", Some("acct"), "New message", "hi")
        .unwrap());
    assert!(bob.insert(&note("a", "slack", "hi", 0), true).unwrap());
}

#[test]
fn dedup_ids_do_not_collide_across_fields() {
    assert_ne!(
        dedup_id("a", None, "b", "c"),
        dedup_id("a", Some(""), "b", "c")
    );
    assert_ne!(
        dedup_id("ab", None, "c", "d"),
        dedup_id("a", None, "bc", "d")
    );
}

#[cfg(unix)]
#[test]
fn workspace_keys_preserve_non_utf8_path_differences() {
    use std::os::unix::ffi::OsStrExt;

    let first = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/tmp/workspace-\x80"));
    let second = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/tmp/workspace-\x81"));

    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    assert_ne!(workspace_key(first), workspace_key(second));
}
