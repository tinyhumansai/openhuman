use super::*;
use crate::openhuman::config::Config;
use serde_json::json;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
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
    use crate::openhuman::task_sources::ops;

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
