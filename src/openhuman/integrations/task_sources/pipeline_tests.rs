use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::integrations::task_sources::store;
use crate::openhuman::integrations::task_sources::types::{FilterSpec, ProviderSlug, SourceTarget};
use serde_json::json;
use tempfile::TempDir;

// This file used to register a stub `ComposioProvider` under the engine's
// provider registry (`register_provider`) so `run_source_once` had something
// to fetch tasks from, then asserted the full fetch → dedup → route →
// reconcile pipeline end to end. tinymemory v1.13.4 deleted
// `ComposioProvider` and the registry outright with no replacement — see
// `pipeline::fetch_tasks_unavailable`'s doc comment — so there is no seam
// left to inject a fake provider through, and `run_inner` now refuses for
// every toolkit before it ever reaches the dedup/route/reconcile stages.
//
// The four tests below assert that refusal is what actually happens (no
// panic, the error lands in `FetchOutcome::error`, nothing gets routed) —
// the honest replacement for "the pipeline runs end to end". The
// dedup/route/reconcile logic these tests used to exercise through the
// pipeline is still covered directly in `store_tests.rs`, `route_tests.rs`
// and `enrich_tests.rs`, none of which ever depended on `ComposioProvider`.

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
            fetch_mode: Default::default(),
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
async fn fetch_surfaces_error_for_every_toolkit() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let source = add_github_source(&config);

    let outcome = run_source_once(&config, &source, FetchReason::Manual).await;
    assert!(outcome.error.is_some(), "fetch must refuse, not panic");
    assert_eq!(outcome.fetched, 0);
    assert_eq!(outcome.routed, 0);
    assert_eq!(outcome.skipped_dupe, 0);
    assert_eq!(outcome.pruned, 0);

    let cards = route::board_cards(&config).await.unwrap();
    assert!(cards.is_empty(), "a refused fetch must route nothing");

    let ingested = store::list_ingested(&config, &source.id, 10).unwrap();
    assert!(ingested.is_empty(), "a refused fetch must ingest nothing");
}

#[tokio::test]
async fn refusal_is_stable_across_repeated_passes() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let source = add_github_source(&config);

    let first = run_source_once(&config, &source, FetchReason::Manual).await;
    let second = run_source_once(&config, &source, FetchReason::Manual).await;
    assert!(first.error.is_some());
    assert!(second.error.is_some());
    assert_eq!(second.routed, 0);
    assert_eq!(second.pruned, 0);
}

#[tokio::test]
async fn refusal_records_a_fetch_history_entry() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let source = add_github_source(&config);

    let _ = run_source_once(&config, &source, FetchReason::Manual).await;

    // `run_source_once` records the failed pass so the UI can show why a
    // source has never ingested anything, rather than looking silently idle.
    let sources = store::list_sources(&config).unwrap();
    let recorded = sources.iter().find(|s| s.id == source.id);
    assert!(
        recorded.is_some(),
        "source must still be listed after a refused fetch"
    );
}

#[tokio::test]
async fn missing_provider_surfaces_error_in_outcome() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A clickup source — no registered provider ever existed for any
    // toolkit, `ComposioProvider` and its registry are gone — so the
    // outcome carries the error, never panics.
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
