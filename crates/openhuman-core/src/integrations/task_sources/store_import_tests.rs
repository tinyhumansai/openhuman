use super::*;
use crate::integrations::composio::contract::NormalizedTask;
use crate::integrations::task_sources::store as live;
use crate::integrations::task_sources::types::{FilterSpec, ProviderSlug, SourceTarget};
use crate::storage::local::{forget, table_names};
use serde_json::json;
use tempfile::TempDir;

fn workspace() -> (TempDir, Config, Config) {
    let dir = TempDir::new().unwrap();
    let mut classic = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    classic.storage.url = Some("classic".into());
    let default = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, classic, default)
}

fn filter() -> FilterSpec {
    FilterSpec::Github {
        repo: Some("tinyhumansai/openhuman".into()),
        labels: vec!["bug".into()],
        assignee_is_me: true,
        state: Some("open".into()),
        fetch_mode: Default::default(),
        extra: json!({}),
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
fn legacy_sources_and_ledger_are_imported_once_through_the_public_api() {
    let (_dir, classic, default) = workspace();
    let first = live::add_source(
        &classic,
        ProviderSlug::Github,
        Some("conn-1".into()),
        Some("My issues".into()),
        filter(),
        1800,
        SourceTarget::AgentTodoProactive,
        25,
    )
    .unwrap();
    let second = live::add_source(
        &classic,
        ProviderSlug::Github,
        None,
        None,
        filter(),
        600,
        SourceTarget::AgentTodoProactive,
        10,
    )
    .unwrap();
    live::mark_ingested(&classic, &first.id, &task("t-1", "First")).unwrap();
    live::mark_ingested(&classic, &first.id, &task("t-2", "Second")).unwrap();
    live::mark_ingested(&classic, &second.id, &task("t-1", "Other")).unwrap();
    live::record_fetch(
        &classic,
        &first.id,
        chrono::Utc::now(),
        crate::integrations::task_sources::FetchReason::Periodic,
        "ok",
    )
    .unwrap();
    let db = live::db_path(&classic);

    let sources = live::list_sources(&default).unwrap();
    assert_eq!(sources.len(), 2);
    let got = live::get_source(&default, &first.id).unwrap();
    assert_eq!(got.name.as_deref(), Some("My issues"));
    assert_eq!(got.connection_id.as_deref(), Some("conn-1"));
    assert_eq!(got.interval_secs, 1800);
    assert_eq!(got.max_tasks_per_fetch, 25);
    assert_eq!(got.filter, filter());
    assert!(got.last_fetch_at.is_some());
    assert_eq!(got.last_status.as_deref(), Some("periodic: ok"));
    assert!(live::was_ingested(&default, &first.id, "t-1").unwrap());
    assert!(live::was_ingested(&default, &second.id, "t-1").unwrap());
    assert!(live::is_ingested(
        &default,
        &first.id,
        "t-1",
        &live::content_hash(&task("t-1", "First"))
    )
    .unwrap());
    let titles: Vec<_> = live::list_ingested(&default, &first.id, 10)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles.len(), 2);
    assert!(titles.contains(&"First".to_string()));
    assert_eq!(
        live::list_ingested_refs(&default, &first.id).unwrap().len(),
        2
    );

    let tables = table_names(&db);
    assert!(tables.contains(&"_legacy_task_sources".to_string()));
    assert!(tables.contains(&"_legacy_ingested_tasks".to_string()));
    assert!(!tables.contains(&"task_sources".to_string()));

    // A restart does not import again.
    live::remove_source(&default, &second.id).unwrap();
    forget(&db);
    assert_eq!(live::list_sources(&default).unwrap().len(), 1);
}

#[test]
fn switching_between_classic_and_default_keeps_working() {
    let (_dir, classic, default) = workspace();
    let add = |config: &Config, interval: u64| {
        live::add_source(
            config,
            ProviderSlug::Github,
            None,
            None,
            filter(),
            interval,
            SourceTarget::AgentTodoProactive,
            10,
        )
        .unwrap()
    };
    add(&classic, 600);
    assert_eq!(live::list_sources(&default).unwrap().len(), 1);
    // The tables were retired; a classic call starts a clean legacy schema
    // (it does not trust a stale version stamp) ...
    assert!(live::list_sources(&classic).unwrap().is_empty());
    add(&classic, 900);
    // ... and the next default call imports what it holds, on top of what the
    // document tables already had.
    assert_eq!(live::list_sources(&default).unwrap().len(), 2);
}

#[test]
fn a_deleted_database_is_reopened_not_served_from_a_stale_handle() {
    let (_dir, classic, default) = workspace();
    live::add_source(
        &classic,
        ProviderSlug::Github,
        None,
        None,
        filter(),
        600,
        SourceTarget::AgentTodoProactive,
        10,
    )
    .unwrap();
    assert_eq!(live::list_sources(&default).unwrap().len(), 1);
    // A data reset removes the workspace while this process keeps running.
    let db = live::db_path(&default);
    for suffix in ["", "-wal", "-shm"] {
        let mut file = db.clone().into_os_string();
        file.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(file));
    }
    assert!(live::list_sources(&default).unwrap().is_empty());
    assert!(db.exists(), "the store was created again");
}

#[test]
fn an_undecodable_source_row_keeps_the_legacy_table_in_service() {
    let (_dir, classic, default) = workspace();
    live::add_source(
        &classic,
        ProviderSlug::Github,
        None,
        None,
        filter(),
        600,
        SourceTarget::AgentTodoProactive,
        10,
    )
    .unwrap();
    let db = live::db_path(&classic);
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "INSERT INTO task_sources (id, provider, enabled, filter, interval_secs, target,
                 max_tasks_per_fetch, created_at)
             VALUES ('bad', 'github', 1, 'not json', 600, '{}', 10, '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
    // The import fails, so nothing is retired and the call reports it.
    let error = live::list_sources(&default).unwrap_err();
    assert!(format!("{error:#}").contains("importing the legacy tables failed"));
    let tables = table_names(&db);
    assert!(tables.contains(&"task_sources".to_string()));
    assert!(!tables.contains(&"_legacy_task_sources".to_string()));
}
