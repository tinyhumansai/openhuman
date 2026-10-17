use super::*;
use chrono::{Datelike, Duration};
use std::fs::{self, OpenOptions};
use std::io::Write;
use tempfile::TempDir;

fn enabled_config() -> CostConfig {
    CostConfig {
        enabled: true,
        ..Default::default()
    }
}

/// A managed-backend tier slug — spend on this route is billed to OpenHuman
/// credits and so is the only kind the local budget may gate (#5016).
const MANAGED_MODEL: &str = "chat-v1";

/// A bring-your-own-key model id, as reported in #5016 (OpenRouter). Spend
/// here is billed by the user's own provider and must never gate a request.
const BYOK_MODEL: &str = "minimax/minimax-m3";

#[test]
fn cost_tracker_initialization() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    assert!(!tracker.session_id().is_empty());
}

#[test]
fn record_usage_and_get_summary() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    let usage = TokenUsage::new("test/model", 1000, 500, 1.0, 2.0);
    tracker.record_usage(usage).unwrap();

    let summary = tracker.get_summary().unwrap();
    assert_eq!(summary.request_count, 1);
    assert!(summary.session_cost_usd > 0.0);
    assert_eq!(summary.by_model.len(), 1);
}

#[test]
fn summary_by_model_is_session_scoped() {
    let tmp = TempDir::new().unwrap();
    let storage_path = resolve_storage_path(tmp.path()).unwrap();
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }

    let old_record = CostRecord::new(
        "old-session",
        TokenUsage::new("legacy/model", 500, 500, 1.0, 1.0),
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(storage_path)
        .unwrap();
    writeln!(file, "{}", serde_json::to_string(&old_record).unwrap()).unwrap();
    file.sync_all().unwrap();

    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    tracker
        .record_usage(TokenUsage::new("session/model", 1000, 1000, 1.0, 1.0))
        .unwrap();

    let summary = tracker.get_summary().unwrap();
    assert_eq!(summary.by_model.len(), 1);
    assert!(summary.by_model.contains_key("session/model"));
    assert!(!summary.by_model.contains_key("legacy/model"));
}

#[test]
fn malformed_lines_are_ignored_while_loading() {
    let tmp = TempDir::new().unwrap();
    let storage_path = resolve_storage_path(tmp.path()).unwrap();
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }

    let valid_usage = TokenUsage::new("test/model", 1000, 0, 1.0, 1.0);
    let valid_record = CostRecord::new("session-a", valid_usage.clone());

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(storage_path)
        .unwrap();
    writeln!(file, "{}", serde_json::to_string(&valid_record).unwrap()).unwrap();
    writeln!(file, "not-a-json-line").unwrap();
    writeln!(file).unwrap();
    file.sync_all().unwrap();

    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let today_cost = tracker.get_daily_cost(Utc::now().date_naive()).unwrap();
    assert!((today_cost - valid_usage.cost_usd).abs() < f64::EPSILON);
}

#[test]
fn record_usage_when_disabled_is_noop() {
    let tmp = TempDir::new().unwrap();
    let config = CostConfig {
        enabled: false,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();
    let usage = TokenUsage::new("test/model", 1000, 500, 1.0, 2.0);
    tracker.record_usage(usage).unwrap();
    let summary = tracker.get_summary().unwrap();
    assert_eq!(summary.request_count, 0);
}

#[test]
fn record_usage_unconditional_bypasses_disabled_gate() {
    let tmp = TempDir::new().unwrap();
    let config = CostConfig {
        enabled: false,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();
    let usage = TokenUsage::new("test/model", 1000, 500, 1.0, 2.0);
    tracker.record_usage_unconditional(usage.clone()).unwrap();
    let summary = tracker.get_summary().unwrap();
    assert_eq!(summary.request_count, 1);
    let today_cost = tracker.get_daily_cost(Utc::now().date_naive()).unwrap();
    assert!((today_cost - usage.cost_usd).abs() < f64::EPSILON);
}

#[test]
fn record_usage_rejects_negative_cost() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let mut usage = TokenUsage::new("test/model", 1000, 500, 1.0, 2.0);
    usage.cost_usd = -1.0;
    assert!(tracker.record_usage(usage).is_err());
}

#[test]
fn record_usage_rejects_nan_cost() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let mut usage = TokenUsage::new("test/model", 1000, 500, 1.0, 2.0);
    usage.cost_usd = f64::NAN;
    assert!(tracker.record_usage(usage).is_err());
}

#[test]
fn byok_spend_is_still_recorded_for_the_dashboard() {
    // Exempting BYOK from the *budget* must not hide it from usage reporting:
    // the user in #5016 explicitly wanted to understand the counter.
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    let mut usage = TokenUsage::new(BYOK_MODEL, 1000, 500, 1.0, 1.0);
    usage.cost_usd = 4.25;
    tracker.record_usage(usage).unwrap();

    let summary = tracker.get_summary().unwrap();
    assert_eq!(summary.request_count, 1);
    assert!((summary.daily_cost_usd - 4.25).abs() < 0.0001);
    assert!((summary.session_cost_usd - 4.25).abs() < 0.0001);
}

#[test]
fn legacy_byok_records_are_exempt_after_an_aggregate_rebuild() {
    // Records persisted by builds that predate #5016 carry no route field. The
    // route is derived from the model id they already store, so a tracker that
    // rebuilds its aggregates from disk classifies them correctly with no
    // migration — this is what unblocks an affected user on upgrade rather
    // than making them wait out the window.
    let tmp = TempDir::new().unwrap();
    let today = Utc::now();
    write_raw_record(tmp.path(), &dated_record("legacy", BYOK_MODEL, 50.0, today));

    let config = CostConfig {
        enabled: true,
        monthly_limit_usd: 10.0,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();

    // Nothing refuses a request on cost any more, so what matters on upgrade is
    // that the legacy rows are classified correctly: they show up in the usage
    // figures without inflating the managed totals the dashboard is drawn
    // against.
    let now = Utc::now();
    let monthly = tracker.get_monthly_cost(now.year(), now.month()).unwrap();
    assert!((monthly - 50.0).abs() < 0.0001);
    let managed_monthly = tracker
        .get_managed_monthly_cost(now.year(), now.month())
        .unwrap();
    assert!(managed_monthly.abs() < f64::EPSILON);
}

#[test]
fn dashboard_budget_gauge_reflects_managed_spend_only() {
    // The phantom "$10/day limit" in the issue was also visible as a budget
    // gauge filling up from BYOK spend against a cap that could never fire.
    let tmp = TempDir::new().unwrap();
    let today = Utc::now();
    write_raw_record(tmp.path(), &dated_record("s1", BYOK_MODEL, 95.0, today));

    let config = CostConfig {
        enabled: true,
        monthly_limit_usd: 100.0,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();
    let dash = tracker.get_dashboard("USD", 0.8, 0.95).unwrap();

    // Usage is still reported…
    assert!((dash.month_to_date_usd - 95.0).abs() < 0.0001);
    assert!((dash.period_total_usd - 95.0).abs() < 0.0001);
    // …but the budget gauge stays empty, because none of it is gateable.
    assert_eq!(dash.budget_status, BudgetStatus::Normal);
    assert!(dash.budget_utilization.abs() < f64::EPSILON);
}

#[test]
fn get_daily_cost_for_today() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let usage = TokenUsage::new("test/model", 1000, 500, 1.0, 2.0);
    tracker.record_usage(usage.clone()).unwrap();

    let today_cost = tracker.get_daily_cost(Utc::now().date_naive()).unwrap();
    assert!((today_cost - usage.cost_usd).abs() < 0.001);
}

#[test]
fn get_monthly_cost_for_current_month() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let usage = TokenUsage::new("test/model", 1000, 500, 1.0, 2.0);
    tracker.record_usage(usage.clone()).unwrap();

    let now = Utc::now();
    let monthly_cost = tracker.get_monthly_cost(now.year(), now.month()).unwrap();
    assert!((monthly_cost - usage.cost_usd).abs() < 0.001);
}

fn write_raw_record(workspace: &Path, record: &CostRecord) {
    let storage_path = resolve_storage_path(workspace).unwrap();
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(storage_path)
        .unwrap();
    writeln!(file, "{}", serde_json::to_string(record).unwrap()).unwrap();
    file.sync_all().unwrap();
}

fn dated_record(session: &str, model: &str, cost: f64, when: chrono::DateTime<Utc>) -> CostRecord {
    let mut usage = TokenUsage::new(model, 1000, 500, 1.0, 1.0);
    usage.cost_usd = cost;
    usage.timestamp = when;
    CostRecord::new(session, usage)
}

#[test]
fn get_daily_history_returns_seven_days_with_gaps_filled() {
    let tmp = TempDir::new().unwrap();
    let today = Utc::now();
    let three_days_ago = today - Duration::days(3);
    let six_days_ago = today - Duration::days(6);

    write_raw_record(
        tmp.path(),
        &dated_record("s1", "model-a", 1.50, three_days_ago),
    );
    write_raw_record(
        tmp.path(),
        &dated_record("s1", "model-b", 0.50, six_days_ago),
    );

    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let history = tracker.get_daily_history(7).unwrap();
    assert_eq!(history.len(), 7);
    // Oldest first → six_days_ago
    assert_eq!(history[0].date, six_days_ago.date_naive());
    assert!((history[0].cost_usd - 0.50).abs() < f64::EPSILON);
    // Three days ago has the second record
    assert_eq!(history[3].date, three_days_ago.date_naive());
    assert!((history[3].cost_usd - 1.50).abs() < f64::EPSILON);
    // Today is the last bucket
    assert_eq!(history[6].date, today.date_naive());
    assert!(history[6].cost_usd.abs() < f64::EPSILON);
    assert_eq!(history[6].request_count, 0);
}

#[test]
fn get_daily_history_excludes_out_of_window_records() {
    let tmp = TempDir::new().unwrap();
    let today = Utc::now();
    let ten_days_ago = today - Duration::days(10);
    write_raw_record(
        tmp.path(),
        &dated_record("s1", "model-a", 99.0, ten_days_ago),
    );

    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let history = tracker.get_daily_history(7).unwrap();
    assert_eq!(history.len(), 7);
    let total: f64 = history.iter().map(|e| e.cost_usd).sum();
    assert!(total.abs() < f64::EPSILON);
}

#[test]
fn get_daily_history_clamps_days_argument() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    assert_eq!(tracker.get_daily_history(0).unwrap().len(), 1);
    assert_eq!(tracker.get_daily_history(367).unwrap().len(), 366);
}

#[test]
fn get_dashboard_computes_period_total_and_monthly_pace() {
    let tmp = TempDir::new().unwrap();
    let today = Utc::now();
    write_raw_record(tmp.path(), &dated_record("s1", "model-a", 2.0, today));
    write_raw_record(
        tmp.path(),
        &dated_record("s1", "model-b", 0.5, today - Duration::days(1)),
    );

    let config = CostConfig {
        enabled: true,
        monthly_limit_usd: 100.0,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();
    let dash = tracker.get_dashboard("USD", 0.8, 0.95).unwrap();
    assert_eq!(dash.days.len(), 7);
    assert!((dash.period_total_usd - 2.5).abs() < 0.0001);
    // daily avg = 2.5/7, monthly pace = avg * 30
    let expected_pace = (2.5 / 7.0) * 30.0;
    assert!((dash.monthly_pace_usd - expected_pace).abs() < 0.0001);
    assert_eq!(dash.currency, "USD");
    // 2.5 spend on 100 budget → 2.5% utilisation, well below 80% warn.
    assert_eq!(dash.budget_status, BudgetStatus::Normal);
}

#[test]
fn get_dashboard_budget_status_warning_and_exceeded() {
    let tmp = TempDir::new().unwrap();
    let today = Utc::now();
    // Managed-route spend: the budget gauge only tracks what can be gated
    // (#5016), so these have to be managed tier ids to move the status.
    write_raw_record(tmp.path(), &dated_record("s1", MANAGED_MODEL, 85.0, today));

    let config = CostConfig {
        enabled: true,
        monthly_limit_usd: 100.0,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();
    let warn_dash = tracker.get_dashboard("USD", 0.8, 0.95).unwrap();
    assert_eq!(warn_dash.budget_status, BudgetStatus::Warning);

    // The legacy ledger is imported once, so the next spend goes through the
    // tracker like any other.
    tracker
        .record_usage_unconditional(dated_record("s1", MANAGED_MODEL, 15.0, today).usage)
        .unwrap();
    let alert_dash = tracker.get_dashboard("USD", 0.8, 0.95).unwrap();
    assert_eq!(alert_dash.budget_status, BudgetStatus::Exceeded);
    assert!((alert_dash.budget_utilization - 1.0).abs() < f64::EPSILON);
}

#[test]
fn get_dashboard_budget_status_normal_when_limit_zero() {
    let tmp = TempDir::new().unwrap();
    let config = CostConfig {
        enabled: true,
        monthly_limit_usd: 0.0,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();
    let dash = tracker.get_dashboard("USD", 0.8, 0.95).unwrap();
    assert_eq!(dash.budget_status, BudgetStatus::Normal);
    assert!(dash.budget_utilization.abs() < f64::EPSILON);
}

#[test]
fn get_dashboard_by_model_is_sorted_desc() {
    let tmp = TempDir::new().unwrap();
    let today = Utc::now();
    write_raw_record(tmp.path(), &dated_record("s1", "model-a", 1.0, today));
    write_raw_record(tmp.path(), &dated_record("s1", "model-b", 5.0, today));
    write_raw_record(tmp.path(), &dated_record("s1", "model-c", 3.0, today));

    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    let dash = tracker.get_dashboard("USD", 0.8, 0.95).unwrap();
    assert_eq!(dash.by_model.len(), 3);
    assert_eq!(dash.by_model[0].model, "model-b");
    assert_eq!(dash.by_model[1].model, "model-c");
    assert_eq!(dash.by_model[2].model, "model-a");
}

#[test]
fn build_session_model_stats_aggregates_correctly() {
    let records = vec![
        CostRecord::new("s1", TokenUsage::new("model-a", 100, 50, 1.0, 1.0)),
        CostRecord::new("s1", TokenUsage::new("model-a", 200, 100, 1.0, 1.0)),
        CostRecord::new("s1", TokenUsage::new("model-b", 300, 150, 1.0, 1.0)),
    ];
    let stats = build_session_model_stats(&records);
    assert_eq!(stats.len(), 2);
    assert_eq!(stats["model-a"].request_count, 2);
    assert_eq!(stats["model-a"].total_tokens, 450);
    assert_eq!(stats["model-b"].request_count, 1);
}

// ── #6482: the usage-log window is `days`, not `days - 1` ───────────────────

/// Persist one record at an explicit age, so a test can sit either side of a
/// cutoff instead of hoping "now" lands somewhere useful.
fn record_aged(tracker: &CostTracker, model: &str, age: Duration) {
    let mut usage = TokenUsage::new(model, 10, 10, 0.01, 0.01);
    usage.timestamp = chrono::Utc::now() - age;
    tracker.record_usage(usage).unwrap();
}

fn models_returned(tracker: &CostTracker, days: u32) -> Vec<String> {
    tracker
        .get_recent_records(days, 1000)
        .unwrap()
        .into_iter()
        .map(|r| r.usage.model)
        .collect()
}

#[test]
fn usage_log_days_1_returns_a_record_written_now() {
    // The reported symptom (#6482): a client asking for a one-day usage log
    // got an empty list, because `now - (1 - 1) days` is `now` and the
    // `timestamp < earliest` filter then rejects everything already written.
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    tracker
        .record_usage(TokenUsage::new(MANAGED_MODEL, 10, 10, 0.01, 0.01))
        .unwrap();

    assert_eq!(
        models_returned(&tracker, 1).len(),
        1,
        "a record written moments ago must appear in the last-1-day log"
    );
}

#[test]
fn usage_log_days_1_window_is_exactly_24_hours() {
    // Pins the cutoff rather than just asserting "not empty": a record 23h old
    // is inside the last day and one 25h old is outside. Asserting only
    // non-emptiness would pass for any window at all once a row exists.
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    record_aged(&tracker, "inside/23h", Duration::hours(23));
    record_aged(&tracker, "outside/25h", Duration::hours(25));

    let models = models_returned(&tracker, 1);
    assert!(
        models.iter().any(|m| m == "inside/23h"),
        "a record 23h old is within the last 24h and must be returned; got {models:?}"
    );
    assert!(
        !models.iter().any(|m| m == "outside/25h"),
        "a record 25h old is outside the last 24h and must not be returned; got {models:?}"
    );
}

#[test]
fn usage_log_window_is_days_not_days_minus_one_at_the_dashboard_default() {
    // The off-by-one was never specific to `days = 1`; that value is just where
    // it became total. The dashboard's own default is 30 (`useCostDashboard`),
    // which silently returned 29 days of history while the UI promised 30.
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    record_aged(&tracker, "day29/inside", Duration::hours(29 * 24 + 12));
    record_aged(&tracker, "day31/outside", Duration::hours(31 * 24));

    let models = models_returned(&tracker, 30);
    assert!(
        models.iter().any(|m| m == "day29/inside"),
        "a record 29.5 days old is within a 30-day window and must be returned; got {models:?}"
    );
    assert!(
        !models.iter().any(|m| m == "day31/outside"),
        "a record 31 days old is outside a 30-day window and must not be returned; got {models:?}"
    );
}

#[test]
fn daily_history_still_counts_calendar_days_inclusive_of_today() {
    // Guard on the sibling this bug was copied from. `get_daily_history`
    // compares dates, so `today - (span - 1)` is correct there and must stay:
    // `days = 1` is today alone, one bucket.
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    tracker
        .record_usage(TokenUsage::new(MANAGED_MODEL, 10, 10, 0.01, 0.01))
        .unwrap();

    let history = tracker.get_daily_history(1).unwrap();
    assert_eq!(history.len(), 1, "days=1 is today alone");
    assert_eq!(
        history[0].date,
        chrono::Utc::now().date_naive(),
        "the single bucket is today"
    );
    assert_eq!(history[0].request_count, 1);
}

#[test]
fn legacy_host_estimated_rows_are_ignored_by_every_read() {
    let tmp = TempDir::new().unwrap();
    let mut real = TokenUsage::new("openrouter/vendor/real-model", 1000, 500, 0.0, 0.0);
    real.cost_usd = 0.25;
    real.cost_source = CostSource::ProviderCharged;
    let duplicate = TokenUsage::new("host:orchestrator", 1000, 500, 0.0, 0.0);
    assert_eq!(duplicate.cost_source, CostSource::Estimated);
    let mut kept_host = TokenUsage::new("host:kept", 40, 10, 0.0, 0.0);
    kept_host.cost_usd = 0.5;
    kept_host.cost_source = CostSource::ProviderCharged;
    for usage in [real, duplicate, kept_host] {
        write_raw_record(tmp.path(), &CostRecord::new("s", usage));
    }

    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    let today = Utc::now().date_naive();
    let day = tracker
        .get_daily_history(1)
        .unwrap()
        .into_iter()
        .find(|entry| entry.date == today)
        .unwrap();
    assert_eq!(
        day.request_count, 2,
        "the estimated host: row is a duplicate"
    );
    assert_eq!(day.total_tokens, 1500 + 50);
    assert!(!day.by_model.contains_key("host:orchestrator"));
    assert!(day.by_model.contains_key("host:kept"));

    let recent = tracker.get_recent_records(1, 100).unwrap();
    assert_eq!(recent.len(), 2);
    assert!(recent.iter().all(|r| r.usage.model != "host:orchestrator"));
}

#[test]
fn empty_ledger_sums_are_not_negative_zero() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    let summary = tracker.get_summary().unwrap();
    assert!(summary.session_cost_usd.is_sign_positive());
    assert!(!serde_json::to_string(&summary).unwrap().contains("-0.0"));

    let dashboard = tracker.get_dashboard("USD", 0.8, 0.95).unwrap();
    assert!(dashboard.period_total_usd.is_sign_positive());
    assert!(dashboard.monthly_pace_usd.is_sign_positive());
    assert!(!serde_json::to_string(&dashboard).unwrap().contains("-0.0"));
}

#[test]
fn the_ledger_dispatches_to_documents_when_a_backend_is_pinned() {
    use crate::storage::{MemoryStorage, Scope, StorageBackend};
    let storage = MemoryStorage::new();
    let docs_for = |scope: &str| {
        super::super::tracker_documents::CostDocs::over(
            &storage.for_scope(&Scope::new(scope).unwrap()).unwrap(),
        )
    };
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    super::super::tracker_documents::with_override(docs_for("alice"), || {
        tracker
            .record_usage(TokenUsage::new(MANAGED_MODEL, 1000, 500, 1.0, 2.0))
            .unwrap();
        tracker
            .record_usage(TokenUsage::new(BYOK_MODEL, 100, 50, 1.0, 2.0))
            .unwrap();
        let summary = tracker.get_summary().unwrap();
        assert!(summary.daily_cost_usd > 0.0);
        assert_eq!(summary.request_count, 2);
        let now = Utc::now();
        assert!(tracker.get_daily_cost(now.date_naive()).unwrap() > 0.0);
        assert!(tracker.get_monthly_cost(now.year(), now.month()).unwrap() > 0.0);
        assert!(
            tracker
                .get_managed_monthly_cost(now.year(), now.month())
                .unwrap()
                > 0.0
        );
        assert_eq!(tracker.get_recent_records(1, 10).unwrap().len(), 2);
    });

    // Another agent's scope starts empty: the period totals are not a shared cache.
    super::super::tracker_documents::with_override(docs_for("bob"), || {
        let summary = tracker.get_summary().unwrap();
        assert_eq!(summary.daily_cost_usd, 0.0);
        assert!(tracker.get_recent_records(1, 10).unwrap().is_empty());
    });

    // Nothing reached the JSONL file.
    assert!(
        !tmp.path().join("state/costs.jsonl").exists()
            || std::fs::read_to_string(tmp.path().join("state/costs.jsonl"))
                .unwrap()
                .is_empty()
    );
}
