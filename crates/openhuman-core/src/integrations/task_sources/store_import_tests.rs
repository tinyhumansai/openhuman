use super::*;
use crate::integrations::composio::contract::NormalizedTask;
use crate::integrations::task_sources::store as live;
use crate::integrations::task_sources::types::{FilterSpec, SourceTarget};
use crate::storage::import::{run_with, Outcome};
use crate::storage::rusqlite::params;
use serde_json::json;
use tempfile::TempDir;

fn workspace() -> (TempDir, Config) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

fn import(config: &Config) -> Result<Outcome> {
    let backend = crate::storage::local::backend_for(&config.workspace_dir).unwrap();
    run_with(&backend, config, &[super::super::import_source()])
        .remove(0)
        .outcome
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

/// Writes a `task_sources` row the way the retired store did.
fn seed_source(
    config: &Config,
    id: &str,
    name: Option<&str>,
    conn_id: Option<&str>,
    interval: i64,
) {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO task_sources
                (id, provider, connection_id, name, enabled, filter, interval_secs, target,
                 max_tasks_per_fetch, created_at, last_fetch_at, last_status)
             VALUES (?1, 'github', ?2, ?3, 1, ?4, ?5, ?6, 25, '2026-01-01T00:00:00+00:00',
                     ?7, ?8)",
            params![
                id,
                conn_id,
                name,
                serde_json::to_string(&filter()).unwrap(),
                interval,
                serde_json::to_string(&SourceTarget::AgentTodoProactive).unwrap(),
                name.map(|_| "2026-01-02T00:00:00+00:00"),
                name.map(|_| "periodic: ok"),
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

fn seed_ingested(config: &Config, source_id: &str, t: &NormalizedTask) {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO ingested_tasks
                (source_id, external_id, content_hash, title, payload, ingested_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '2026-01-03T00:00:00+00:00')",
            params![
                source_id,
                t.external_id,
                live::content_hash(t),
                t.title,
                serde_json::to_string(t).unwrap(),
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn legacy_sources_and_ledger_are_imported_once_and_the_file_is_left_alone() {
    let (_dir, config) = workspace();
    seed_source(&config, "src-1", Some("My issues"), Some("conn-1"), 1800);
    seed_source(&config, "src-2", None, None, 600);
    seed_ingested(&config, "src-1", &task("t-1", "First"));
    seed_ingested(&config, "src-1", &task("t-2", "Second"));
    seed_ingested(&config, "src-2", &task("t-1", "Other"));
    let db = db_path(&config);
    let before = std::fs::read(&db).unwrap();

    assert!(matches!(
        import(&config).unwrap(),
        Outcome::Imported {
            found: 5,
            written: 5
        }
    ));

    let sources = live::list_sources(&config).unwrap();
    assert_eq!(sources.len(), 2);
    let got = live::get_source(&config, "src-1").unwrap();
    assert_eq!(got.name.as_deref(), Some("My issues"));
    assert_eq!(got.connection_id.as_deref(), Some("conn-1"));
    assert_eq!(got.interval_secs, 1800);
    assert_eq!(got.max_tasks_per_fetch, 25);
    assert_eq!(got.filter, filter());
    assert!(got.last_fetch_at.is_some());
    assert_eq!(got.last_status.as_deref(), Some("periodic: ok"));
    assert!(live::was_ingested(&config, "src-1", "t-1").unwrap());
    assert!(live::was_ingested(&config, "src-2", "t-1").unwrap());
    assert!(live::is_ingested(
        &config,
        "src-1",
        "t-1",
        &live::content_hash(&task("t-1", "First"))
    )
    .unwrap());
    let titles: Vec<_> = live::list_ingested(&config, "src-1", 10)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles.len(), 2);
    assert!(titles.contains(&"First".to_string()));
    assert_eq!(live::list_ingested_refs(&config, "src-1").unwrap().len(), 2);
    assert_eq!(std::fs::read(&db).unwrap(), before, "sources.db untouched");

    // A second run does not import again.
    live::remove_source(&config, "src-2").unwrap();
    assert!(matches!(import(&config).unwrap(), Outcome::AlreadyDone));
    assert_eq!(live::list_sources(&config).unwrap().len(), 1);
}

#[test]
fn an_undecodable_source_row_fails_the_import_and_it_is_retried() {
    let (_dir, config) = workspace();
    seed_source(&config, "good", Some("ok"), None, 600);
    with_connection(&config, |conn| {
        conn.execute(
            "INSERT INTO task_sources (id, provider, enabled, filter, interval_secs, target,
                 max_tasks_per_fetch, created_at)
             VALUES ('bad', 'github', 1, 'not json', 600, '{}', 10, '2026-01-01T00:00:00Z')",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let error = import(&config).unwrap_err();
    assert!(
        format!("{error:#}").contains("decode a task_sources row"),
        "{error:#}"
    );
    // No marker was written, so nothing is imported and the next boot retries.
    assert!(live::list_sources(&config).unwrap().is_empty());
}
