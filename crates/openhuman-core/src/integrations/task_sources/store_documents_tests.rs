use super::*;
use crate::storage::{MemoryStorage, Scope, StorageBackend};

fn docs_in(storage: &MemoryStorage, scope: &str) -> Docs {
    Docs::over(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn docs() -> Docs {
    docs_in(&MemoryStorage::new(), "local")
}

fn github_filter() -> FilterSpec {
    FilterSpec::Github {
        repo: Some("tinyhumansai/openhuman".into()),
        labels: vec!["bug".into()],
        assignee_is_me: true,
        state: Some("open".into()),
        fetch_mode: Default::default(),
        extra: json!({}),
    }
}

fn source(id: &str) -> TaskSource {
    TaskSource {
        id: id.to_string(),
        provider: ProviderSlug::Github,
        connection_id: None,
        name: Some("My issues".into()),
        enabled: true,
        filter: github_filter(),
        interval_secs: 1800,
        target: SourceTarget::AgentTodoProactive,
        max_tasks_per_fetch: 25,
        created_at: Utc::now(),
        last_fetch_at: None,
        last_status: None,
    }
}

fn task(external_id: &str, title: &str) -> NormalizedTask {
    NormalizedTask {
        external_id: external_id.into(),
        provider: "github".into(),
        title: title.into(),
        ..Default::default()
    }
}

#[test]
fn sources_round_trip_oldest_first() {
    let store = docs();
    let mut older = source("b");
    older.created_at = Utc::now() - chrono::Duration::seconds(10);
    let first = store.add_source(&older).unwrap();
    assert_eq!(first, source_with_time("b", first.created_at));
    store.add_source(&source("a")).unwrap();
    let ids: Vec<String> = store
        .list_sources()
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, ["b", "a"]);
    assert!(
        store.add_source(&source("a")).is_err(),
        "an id is added once"
    );
    let missing = store.get_source("nope").unwrap_err();
    assert!(missing.to_string().contains("not found"));
}

fn source_with_time(id: &str, created_at: DateTime<Utc>) -> TaskSource {
    TaskSource {
        created_at,
        ..source(id)
    }
}

#[test]
fn update_applies_the_patch_and_validates_it() {
    let store = docs();
    store.add_source(&source("s")).unwrap();
    let updated = store
        .update_source(
            "s",
            TaskSourcePatch {
                name: Some("   ".into()),
                enabled: Some(false),
                interval_secs: Some(60),
                connection_id: Some("conn-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(updated.name.is_none(), "a blank name clears it");
    assert!(!updated.enabled);
    assert_eq!(updated.interval_secs, 60);
    assert_eq!(updated.connection_id.as_deref(), Some("conn-1"));
    assert_eq!(store.get_source("s").unwrap(), updated);

    let notion = FilterSpec::Notion {
        database_id: None,
        assigned_to_me: false,
        status: None,
        extra: json!({}),
    };
    let error = store
        .update_source(
            "s",
            TaskSourcePatch {
                filter: Some(notion),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("does not match"), "{error}");
    assert!(store
        .update_source("nope", TaskSourcePatch::default())
        .is_err());
}

#[test]
fn record_fetch_stamps_the_source() {
    let store = docs();
    store.add_source(&source("s")).unwrap();
    let at = Utc::now();
    store
        .record_fetch("s", at, FetchReason::Manual, "ok: 3 new")
        .unwrap();
    store
        .record_fetch("missing", at, FetchReason::Manual, "ok")
        .unwrap();
    let read = store.get_source("s").unwrap();
    // Stored as epoch milliseconds.
    assert_eq!(
        read.last_fetch_at.map(|at| at.timestamp_millis()),
        Some(at.timestamp_millis())
    );
    assert_eq!(
        read.last_status.as_deref(),
        Some(format!("{}: ok: 3 new", FetchReason::Manual.as_str()).as_str())
    );
}

#[test]
fn the_ingest_ledger_is_edit_aware() {
    let store = docs();
    store.add_source(&source("s")).unwrap();
    let original = task("1", "Fix it");
    assert!(!store.was_ingested("s", "1").unwrap());
    store.mark_ingested("s", &original).unwrap();
    assert!(store.was_ingested("s", "1").unwrap());
    assert!(store
        .is_ingested("s", "1", &content_hash(&original))
        .unwrap());
    let edited = task("1", "Fix it properly");
    assert!(!store.is_ingested("s", "1", &content_hash(&edited)).unwrap());
    store.mark_ingested("s", &edited).unwrap();
    assert!(store.is_ingested("s", "1", &content_hash(&edited)).unwrap());
    assert_eq!(store.list_ingested("s", 10).unwrap().len(), 1, "an upsert");
}

#[test]
fn ledger_lists_refs_oldest_first_and_tasks_newest_first() {
    let store = docs();
    store.add_source(&source("s")).unwrap();
    let base = Utc::now();
    store
        .mark_ingested_at("s", &task("1", "one"), base - chrono::Duration::seconds(10))
        .unwrap();
    store
        .mark_ingested_at("s", &task("2", "two"), base)
        .unwrap();
    store.add_source(&source("other")).unwrap();
    store.mark_ingested("other", &task("9", "nine")).unwrap();
    let refs: Vec<String> = store
        .list_ingested_refs("s")
        .unwrap()
        .into_iter()
        .map(|r| r.external_id)
        .collect();
    assert_eq!(refs, ["1", "2"]);
    let titles: Vec<String> = store
        .list_ingested("s", 0)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles, ["two"], "a zero limit still returns one");
    assert!(store.remove_ingested("s", "1").unwrap());
    assert!(!store.remove_ingested("s", "1").unwrap());
}

#[test]
fn removing_a_source_drops_its_ledger() {
    let store = docs();
    store.add_source(&source("s")).unwrap();
    store.add_source(&source("t")).unwrap();
    store.mark_ingested("s", &task("1", "one")).unwrap();
    store.mark_ingested("t", &task("1", "one")).unwrap();
    store.remove_source("s").unwrap();
    assert!(store.remove_source("s").is_err(), "already gone");
    assert!(!store.was_ingested("s", "1").unwrap());
    assert!(store.was_ingested("t", "1").unwrap());
    assert_eq!(store.clear_all().unwrap(), 1);
    assert!(store.list_sources().unwrap().is_empty());
    assert!(!store.was_ingested("t", "1").unwrap());
}

#[test]
fn a_corrupt_source_is_an_error_not_a_panic() {
    let store = docs();
    store
        .0
        .run(|docs| async move {
            docs.put(
                SOURCES,
                "bad",
                json!({ "provider": "github" }),
                Precondition::Absent,
            )
            .await
            .map(|_| ())
        })
        .unwrap();
    assert!(store.get_source("bad").is_err());
    assert!(store.list_sources().is_err());
}

#[test]
fn scopes_keep_sources_apart() {
    let storage = MemoryStorage::new();
    let alice = docs_in(&storage, "alice");
    alice.add_source(&source("s")).unwrap();
    alice.mark_ingested("s", &task("1", "one")).unwrap();
    let bob = docs_in(&storage, "bob");
    assert!(bob.list_sources().unwrap().is_empty());
    assert!(!bob.was_ingested("s", "1").unwrap());
    assert_ne!(ingested_id("a/b", "c"), ingested_id("a", "b/c"));
}

#[test]
fn a_ledger_write_needs_its_source() {
    let store = docs();
    assert!(store.mark_ingested("ghost", &task("1", "one")).is_err());
    assert!(!store.was_ingested("ghost", "1").unwrap());
}

#[test]
fn a_corrupt_ledger_payload_is_an_error() {
    let store = docs();
    store.add_source(&source("s")).unwrap();
    store
        .0
        .run(|docs| async move {
            docs.put(
                INGESTED,
                "bad",
                json!({ "source_id": "s", "payload": "not json", "ingested_ms": 1 }),
                Precondition::Absent,
            )
            .await
            .map(|_| ())
        })
        .unwrap();
    assert!(store.list_ingested("s", 10).is_err());
}
