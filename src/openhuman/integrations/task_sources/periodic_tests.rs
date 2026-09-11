use super::*;
use crate::openhuman::integrations::task_sources::types::{FilterSpec, ProviderSlug, SourceTarget};
use chrono::Utc;
use serde_json::json;

fn source(id: &str, interval_secs: u64) -> TaskSource {
    source_for(id, interval_secs, ProviderSlug::Github)
}

fn source_for(id: &str, interval_secs: u64, provider: ProviderSlug) -> TaskSource {
    TaskSource {
        id: id.into(),
        provider,
        connection_id: None,
        name: None,
        enabled: true,
        filter: FilterSpec::Github {
            repo: None,
            labels: vec![],
            assignee_is_me: true,
            state: None,
            fetch_mode: Default::default(),
            extra: json!({}),
        },
        interval_secs,
        target: SourceTarget::TodoOnly,
        max_tasks_per_fetch: 25,
        assigned_executor: None,
        created_at: Utc::now(),
        last_fetch_at: None,
        last_status: None,
    }
}

#[test]
fn tick_seconds_is_sane() {
    assert!(TICK_SECONDS >= 60);
    assert!(TICK_SECONDS <= 3600);
}

#[test]
fn never_polled_source_is_due() {
    let s = source("ts-never-polled-xyz", 1800);
    assert!(is_due(&s));
}

#[test]
fn recently_polled_source_is_not_due() {
    let s = source("ts-recent-poll-xyz", 1800);
    record_poll(&s.id);
    assert!(!is_due(&s), "just-recorded poll should not be due again");
}

#[test]
fn zero_interval_is_floored_not_always_due() {
    let s = source("ts-zero-interval-xyz", 0);
    record_poll(&s.id);
    // With the MIN_INTERVAL_SECONDS floor a just-polled zero-interval
    // source is not immediately due again.
    assert!(!is_due(&s));
}

// ── the containment gate (issue #6118) ──────────────────────────────────────
//
// Two halves of one distinction, pinned together on purpose: the timer must
// stay silent about a provider it cannot fetch, and the manual RPC must keep
// explaining itself. A change that silences both would satisfy either test
// alone.

/// No provider has a task-fetch path today, so the scheduler has nothing
/// legitimate to poll. When one is restored this test is what tells you to
/// update it — deliberately, rather than by a blanket assertion that would
/// keep passing.
#[test]
fn no_provider_can_fetch_today() {
    for provider in [
        ProviderSlug::Github,
        ProviderSlug::Notion,
        ProviderSlug::Linear,
        ProviderSlug::Clickup,
    ] {
        assert!(
            !provider.can_fetch(),
            "{} has no fetch path until the pipeline's fetch step is restored",
            provider.as_str()
        );
    }
}

/// Drives a real `run_one_tick` over a persisted, enabled, due source whose
/// provider cannot fetch, and asserts the tick recorded **nothing**.
///
/// The observable is the source row itself: the failure arm of
/// `pipeline::run_source_once` calls `store::record_fetch`, which stamps
/// `last_fetch_at` and `last_status`. Both staying `None` after a tick is
/// proof the source was skipped before `run_source_once` ran.
///
/// This asserts through the scheduler rather than on the predicate, on
/// purpose: a test that only checked `can_fetch()` would still pass if the
/// guard were deleted from `run_one_tick`, which is the exact regression this
/// PR exists to prevent.
///
/// The config is scoped onto the embedder seam that `load_config_with_timeout`
/// prefers, so the tick reads this workspace instead of doing live, racy disk
/// I/O against `~/.openhuman` — the same approach as `sandbox/schemas_tests.rs`.
#[tokio::test]
async fn a_tick_over_an_unfetchable_source_records_nothing() {
    use crate::core::runtime::context::CoreContext;
    use crate::core::runtime::DomainSet;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let config = crate::openhuman::config::Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    assert!(
        config.task_sources.enabled,
        "precondition: the domain must be on, or the tick returns before the loop"
    );

    // `add_source` mints the id, and it is fresh per run — so the
    // process-global last-poll map cannot make this source look recently
    // polled because of another test.
    let stored = super::super::store::add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        FilterSpec::Github {
            repo: None,
            labels: vec![],
            assignee_is_me: true,
            state: None,
            fetch_mode: Default::default(),
            extra: json!({}),
        },
        1800,
        SourceTarget::TodoOnly,
        25,
    )
    .expect("persist the source");
    let id = stored.id.clone();

    assert!(stored.enabled, "precondition: the source is enabled");
    assert!(is_due(&stored), "precondition: the source is due");
    assert!(
        !stored.provider.can_fetch(),
        "precondition: its provider has no fetch path, so the gate must skip it"
    );

    CoreContext::scope(
        CoreContext::for_test_with_config(DomainSet::full(), config.clone()),
        run_one_tick(),
    )
    .await
    .expect("a tick must not fail");

    let after = super::super::store::get_source(&config, &id).expect("re-read the source");
    assert!(
        after.last_fetch_at.is_none(),
        "the tick must not have stamped a fetch time: {:?}",
        after.last_fetch_at
    );
    assert!(
        after.last_status.is_none(),
        "and must not have recorded a status — a recorded failure is the bug: {:?}",
        after.last_status
    );
}

/// The other half: the manual path is untouched. `run_source_once` — which the
/// `task_sources_fetch` RPC calls directly, without the scheduler's gate —
/// still refuses and still says why.
///
/// This is what makes the gate a scheduler-only change rather than a silent
/// disabling of the feature.
#[tokio::test]
async fn manual_fetch_still_returns_the_explanatory_error() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let config = crate::openhuman::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");

    let s = source("ts-manual-path-xyz", 1800);
    let outcome = pipeline::run_source_once(&config, &s, FetchReason::Manual).await;

    let error = outcome
        .error
        .as_deref()
        .expect("the manual path must still surface an error, not silently succeed");
    assert!(
        error.contains("unavailable"),
        "and it must still explain itself: {error}"
    );
    assert_eq!(outcome.fetched, 0);
    assert_eq!(outcome.routed, 0);
}
