use super::*;
use crate::openhuman::config::Config;
use serde_json::json;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
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

fn sample_task(external_id: &str, title: &str, updated: &str) -> NormalizedTask {
    NormalizedTask {
        external_id: external_id.into(),
        source_id: String::new(),
        provider: "github".into(),
        title: title.into(),
        updated_at: Some(updated.into()),
        ..Default::default()
    }
}

#[test]
fn add_get_and_list_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        Some("My issues".into()),
        github_filter(),
        1800,
        SourceTarget::AgentTodoProactive,
        25,
    )
    .unwrap();
    assert!(!src.id.is_empty());
    assert_eq!(src.provider, ProviderSlug::Github);
    assert!(src.enabled);

    let fetched = get_source(&config, &src.id).unwrap();
    assert_eq!(fetched, src);

    let all = list_sources(&config).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, src.id);
}

#[test]
fn add_rejects_provider_filter_mismatch() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = add_source(
        &config,
        ProviderSlug::Notion,
        None,
        None,
        github_filter(), // github filter under a notion source
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap_err();
    assert!(err.to_string().contains("does not match"));
}

#[test]
fn update_applies_partial_patch() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        1800,
        SourceTarget::AgentTodoProactive,
        25,
    )
    .unwrap();

    let patched = update_source(
        &config,
        &src.id,
        TaskSourcePatch {
            enabled: Some(false),
            interval_secs: Some(600),
            target: Some(SourceTarget::TodoOnly),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!patched.enabled);
    assert_eq!(patched.interval_secs, 600);
    assert_eq!(patched.target, SourceTarget::TodoOnly);
    // Untouched fields preserved.
    assert_eq!(patched.filter, src.filter);
}

#[test]
fn update_rejects_cross_provider_filter() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();
    let err = update_source(
        &config,
        &src.id,
        TaskSourcePatch {
            filter: Some(FilterSpec::Notion {
                database_id: None,
                assigned_to_me: true,
                status: None,
                extra: json!({}),
            }),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("does not match"));
}

#[test]
fn remove_deletes_and_cascades_ingested() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();
    mark_ingested(
        &config,
        &src.id,
        &sample_task("1", "A", "2025-01-01"),
        "task-abc",
    )
    .unwrap();

    remove_source(&config, &src.id).unwrap();
    assert!(get_source(&config, &src.id).is_err());
    // Ingested rows cascade-deleted.
    assert!(list_ingested(&config, &src.id, 10).unwrap().is_empty());
    // Removing again errors (not found).
    assert!(remove_source(&config, &src.id).is_err());
}

#[test]
fn dedup_detects_seen_and_edited_tasks() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();

    let task = sample_task("42", "Fix bug", "2025-01-01T00:00:00Z");
    let hash = content_hash(&task);
    // Not ingested yet.
    assert!(!is_ingested(&config, &src.id, "42", &hash).unwrap());

    mark_ingested(&config, &src.id, &task, "task-v1").unwrap();
    // Same content hash → already ingested.
    assert!(is_ingested(&config, &src.id, "42", &hash).unwrap());

    // Edited task (newer updated_at) → different hash → not ingested.
    let edited = sample_task("42", "Fix bug", "2025-02-01T00:00:00Z");
    let edited_hash = content_hash(&edited);
    assert_ne!(hash, edited_hash);
    assert!(!is_ingested(&config, &src.id, "42", &edited_hash).unwrap());

    // Re-ingesting the edit upserts (still one row).
    mark_ingested(&config, &src.id, &edited, "task-v2").unwrap();
    let listed = list_ingested(&config, &src.id, 10).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].external_id, "42");
}

#[tokio::test]
async fn add_with_assigned_executor_persists_and_filters_blank() {
    use crate::openhuman::integrations::task_sources::ops;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Some(non-empty) → persisted via the follow-up update_source patch
    // (exercises both ops::add's assigned-executor branch and store's
    // update_source patch arm). The store layer preserves the value verbatim;
    // route::add_card is what trims it when stamping a card's assigned_agent.
    let out = ops::add(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        Some(1800),
        Some(SourceTarget::TodoOnly),
        Some(25),
        Some("my-skill".into()),
    )
    .await
    .expect("add with executor");
    assert_eq!(out.value.assigned_executor.as_deref(), Some("my-skill"));

    // Re-read from disk to confirm persistence (not just the returned value).
    let fetched = get_source(&config, &out.value.id).unwrap();
    assert_eq!(fetched.assigned_executor.as_deref(), Some("my-skill"));

    // Whitespace-only executor is filtered to None before the patch runs.
    let blank = ops::add(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        Some(1800),
        Some(SourceTarget::TodoOnly),
        Some(25),
        Some("   ".into()),
    )
    .await
    .expect("add with blank executor");
    assert_eq!(blank.value.assigned_executor, None);
}

#[tokio::test]
async fn ops_remove_prunes_routed_cards_for_source() {
    use crate::openhuman::integrations::task_sources::{ops, route};
    use crate::openhuman::threads::todos::ops::{add as todo_add, BoardLocation, CardPatch};

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();
    let location = BoardLocation::Thread {
        workspace_dir: config.workspace_dir.clone(),
        thread_id: route::TASK_SOURCES_THREAD_ID.to_string(),
    };
    let snapshot = todo_add(&location, "[GitHub] A", CardPatch::default())
        .await
        .unwrap();
    let card_id = snapshot.cards.last().unwrap().id.clone();
    mark_ingested(
        &config,
        &src.id,
        &sample_task("1", "A", "2025-01-01"),
        &card_id,
    )
    .unwrap();

    let out = ops::remove(&config, &src.id).await.expect("remove source");
    assert_eq!(out.value["removed"], true);
    assert_eq!(out.value["pruned"], 1);
    assert!(route::board_cards(&config).await.unwrap().is_empty());
    assert!(list_ingested(&config, &src.id, 10).unwrap().is_empty());
}

#[test]
fn content_hash_changes_when_only_url_changes() {
    // `url` is load-bearing downstream (source_metadata / external write-back),
    // so a URL-only upstream edit must produce a different hash and re-ingest —
    // even if `updated_at` didn't advance (coarse-`updated_at` providers).
    let base = sample_task("7", "Same title", "2025-01-01T00:00:00Z");
    let mut moved = base.clone();
    moved.url = Some("https://example.com/issues/7".into());
    assert_ne!(
        content_hash(&base),
        content_hash(&moved),
        "a URL-only change must re-ingest"
    );
}

#[test]
fn list_ingested_orders_newest_first() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();

    mark_ingested(
        &config,
        &src.id,
        &sample_task("1", "first", "2025-01-01"),
        "task-1",
    )
    .unwrap();
    mark_ingested(
        &config,
        &src.id,
        &sample_task("2", "second", "2025-01-02"),
        "task-2",
    )
    .unwrap();
    let listed = list_ingested(&config, &src.id, 10).unwrap();
    assert_eq!(listed.len(), 2);
    // Newest ingested_at first; "2" was inserted last.
    assert_eq!(listed[0].external_id, "2");
}

#[test]
fn clear_all_removes_every_source() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();
    let removed = clear_all(&config).unwrap();
    assert_eq!(removed, 1);
    assert!(list_sources(&config).unwrap().is_empty());
}

/// Regression: gating the DDL behind a per-path "already initialized" set
/// (see [`INITIALIZED_SCHEMAS`]) must not cost the store its self-healing.
///
/// Before the gate existed, the DDL ran on every `with_connection` call, so a
/// database deleted or replaced at runtime (a workspace reset, a manual
/// deletion, a disk-recovery restore) recovered on the very next call —
/// `Connection::open` creates a fresh empty file and `CREATE TABLE IF NOT
/// EXISTS` repopulates it. With a naive cache the set still reports
/// "initialized" while the file behind it is empty, and every query afterwards
/// fails `no such table: task_sources` until the process restarts. This pins
/// the verify-on-hit in `ensure_schema_initialized` that restores it.
#[test]
fn schema_reinitializes_when_the_database_file_is_deleted_at_runtime() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // First use populates the per-path cache and creates the schema.
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        Some("before-deletion".into()),
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();
    assert_eq!(
        list_sources(&config).unwrap().len(),
        1,
        "sanity: the source was persisted"
    );

    // Simulate a workspace reset / manual deletion while the process lives on.
    let db_path = config.workspace_dir.join("task_sources").join("sources.db");
    assert!(
        db_path.exists(),
        "sanity: the task_sources db exists before deletion"
    );
    std::fs::remove_file(&db_path).unwrap();
    // This store runs in WAL mode, so the sidecars must go too or SQLite can
    // resurrect pages from them.
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));

    // The cache still says this path is initialized. Without the verify-on-hit
    // this errors with `no such table: task_sources`.
    let after = list_sources(&config)
        .expect("a deleted database must be re-initialized, not left wedged at 'no such table'");
    assert!(
        after.is_empty(),
        "the recreated database starts empty — the prior source is genuinely gone"
    );
    // The prior source's id no longer resolves against the fresh db.
    assert!(get_source(&config, &src.id).is_err());

    // And the store is fully usable again, not merely readable.
    let recreated = add_source(
        &config,
        ProviderSlug::Github,
        None,
        Some("after-deletion".into()),
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();
    assert_eq!(get_source(&config, &recreated.id).unwrap().id, recreated.id);
    assert_eq!(list_sources(&config).unwrap().len(), 1);
}

/// Regression (CodeRabbit / Codex on #5709): a cache hit must be validated
/// against the *whole* schema, not just the presence of one table. A database
/// replaced at runtime with an older/partial schema — `task_sources` present but
/// a migrated column dropped — must be re-migrated, not trusted and then failed
/// on the incomplete schema. The `PRAGMA user_version` check is what catches it.
#[test]
fn older_on_disk_schema_under_a_cached_path_is_remigrated() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // First use creates the full (versioned) schema and caches the path.
    let original = add_source(
        &config,
        ProviderSlug::Github,
        None,
        Some("v1".into()),
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();

    // Simulate a workspace restore of an OLDER database swapped in under the
    // same (already-cached) path: drop a migrated column and clear the version
    // stamp, exactly as a pre-migration database would look on disk.
    let db_path = config.workspace_dir.join("task_sources").join("sources.db");
    {
        let raw = rusqlite::Connection::open(&db_path).unwrap();
        raw.execute_batch(
            "ALTER TABLE task_sources DROP COLUMN assigned_executor;
             PRAGMA user_version = 0;",
        )
        .unwrap();
    }

    // The path is still cached. With a single-table `sqlite_master` probe this
    // would be trusted and `list_sources` (which selects `assigned_executor`)
    // would fail with `no such column`. The version check detects the drift and
    // re-migrates instead.
    let listed = list_sources(&config)
        .expect("an older on-disk schema under a cached path must be re-migrated, not trusted");
    assert_eq!(
        listed.len(),
        1,
        "the pre-existing row survives DROP COLUMN and the schema is repaired"
    );
    assert_eq!(listed[0].id, original.id);
    // The migrated column is back (reads as None for the pre-existing row).
    assert_eq!(
        get_source(&config, &original.id).unwrap().assigned_executor,
        None
    );

    // And the store is fully usable again.
    let src = add_source(
        &config,
        ProviderSlug::Github,
        None,
        Some("v2".into()),
        github_filter(),
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();
    assert_eq!(get_source(&config, &src.id).unwrap().id, src.id);
}
