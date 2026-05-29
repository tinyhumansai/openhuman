use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::{
    register_provider, ComposioProvider, NormalizedTask, ProviderContext, ProviderUserProfile,
    SyncOutcome, SyncReason, TaskFetchFilter,
};
use crate::openhuman::task_sources::store;
use crate::openhuman::task_sources::types::{FilterSpec, ProviderSlug, SourceTarget};
use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

/// Serialize pipeline tests: they register a stub provider under the
/// shared "github" registry slug, so they must not run concurrently.
fn registry_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

struct StubProvider {
    tasks: Vec<NormalizedTask>,
}

#[async_trait]
impl ComposioProvider for StubProvider {
    fn toolkit_slug(&self) -> &'static str {
        "github"
    }
    async fn fetch_user_profile(
        &self,
        _ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        Ok(ProviderUserProfile::default())
    }
    async fn sync(
        &self,
        _ctx: &ProviderContext,
        _reason: SyncReason,
    ) -> Result<SyncOutcome, String> {
        Ok(SyncOutcome::default())
    }
    async fn fetch_tasks(
        &self,
        _ctx: &ProviderContext,
        _filter: &TaskFetchFilter,
    ) -> Result<Vec<NormalizedTask>, String> {
        Ok(self.tasks.clone())
    }
}

fn canned_task(id: &str, title: &str, updated: &str) -> NormalizedTask {
    NormalizedTask {
        external_id: id.into(),
        source_id: String::new(),
        provider: "github".into(),
        title: title.into(),
        url: Some(format!("https://example.com/{id}")),
        updated_at: Some(updated.into()),
        ..Default::default()
    }
}

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn add_github_source(config: &Config) -> TaskSource {
    store::add_source(
        config,
        ProviderSlug::Github,
        None,
        Some("Test source".into()),
        FilterSpec::Github {
            repo: Some("o/r".into()),
            labels: vec![],
            assignee_is_me: true,
            state: None,
            extra: json!({}),
        },
        1800,
        // TodoOnly keeps the pass deterministic — no triage LLM turn.
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap()
}

#[tokio::test]
async fn fetch_routes_cards_and_dedups_on_rerun() {
    let _guard = registry_lock();
    register_provider(Arc::new(StubProvider {
        tasks: vec![
            canned_task("1", "First task", "2025-01-01T00:00:00Z"),
            canned_task("2", "Second task", "2025-01-02T00:00:00Z"),
        ],
    }));

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let source = add_github_source(&config);

    // First pass: both tasks fetched and routed onto the board.
    let outcome = run_source_once(&config, &source, FetchReason::Manual).await;
    assert_eq!(outcome.fetched, 2, "error={:?}", outcome.error);
    assert_eq!(outcome.routed, 2);
    assert_eq!(outcome.skipped_dupe, 0);
    assert!(outcome.error.is_none());

    let cards = route::board_cards(&config).unwrap();
    assert_eq!(cards.len(), 2);
    assert!(cards.iter().any(|c| c.title.contains("First task")));
    assert!(cards.iter().all(|c| c.title.starts_with("[GitHub]")));

    // Second pass: same tasks → all deduped, no new cards.
    let outcome2 = run_source_once(&config, &source, FetchReason::Manual).await;
    assert_eq!(outcome2.fetched, 2);
    assert_eq!(outcome2.routed, 0);
    assert_eq!(outcome2.skipped_dupe, 2);

    let cards_after = route::board_cards(&config).unwrap();
    assert_eq!(cards_after.len(), 2, "dedup must not add duplicate cards");

    // Ingested ledger reflects both tasks.
    let ingested = store::list_ingested(&config, &source.id, 10).unwrap();
    assert_eq!(ingested.len(), 2);
}

#[tokio::test]
async fn edited_task_reroutes_as_new_card() {
    let _guard = registry_lock();
    register_provider(Arc::new(StubProvider {
        tasks: vec![canned_task("7", "Original title", "2025-01-01T00:00:00Z")],
    }));

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let source = add_github_source(&config);

    let first = run_source_once(&config, &source, FetchReason::Manual).await;
    assert_eq!(first.routed, 1);

    // Re-register with an edited version (newer updated_at → new hash).
    register_provider(Arc::new(StubProvider {
        tasks: vec![canned_task("7", "Edited title", "2025-02-01T00:00:00Z")],
    }));
    let second = run_source_once(&config, &source, FetchReason::Manual).await;
    assert_eq!(second.routed, 1, "edited task should re-route");
    assert_eq!(second.skipped_dupe, 0);

    // Board must have exactly one card: the stale card was removed before
    // the fresh one was added, so no duplicate accumulation.
    let cards = route::board_cards(&config).unwrap();
    assert_eq!(
        cards.len(),
        1,
        "edited task must not leave duplicate board cards"
    );
    assert!(cards[0].title.contains("Edited title"));

    // Ledger still holds a single row for external_id 7 (upsert).
    let ingested = store::list_ingested(&config, &source.id, 10).unwrap();
    assert_eq!(ingested.len(), 1);
    assert_eq!(ingested[0].title, "Edited title");
}

#[tokio::test]
async fn missing_provider_surfaces_error_in_outcome() {
    let _guard = registry_lock();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A clickup source with no registered clickup provider in the test
    // binary → outcome carries the error, never panics.
    let source = store::add_source(
        &config,
        ProviderSlug::Clickup,
        None,
        None,
        FilterSpec::Clickup {
            team_id: None,
            list_id: None,
            assignee_is_me: true,
            extra: json!({}),
        },
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .unwrap();

    let outcome = run_source_once(&config, &source, FetchReason::Manual).await;
    assert!(outcome.error.is_some());
    assert_eq!(outcome.routed, 0);
}
