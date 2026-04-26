use super::*;
use tempfile::TempDir;

fn enabled_config() -> CostConfig {
    CostConfig {
        enabled: true,
        ..Default::default()
    }
}

#[test]
fn cost_tracker_initialization() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    assert!(!tracker.session_id().is_empty());
}

#[test]
fn budget_check_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = CostConfig {
        enabled: false,
        ..Default::default()
    };

    let tracker = CostTracker::new(config, tmp.path()).unwrap();
    let check = tracker.check_budget(1000.0).unwrap();
    assert!(matches!(check, BudgetCheck::Allowed));
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
fn budget_exceeded_daily_limit() {
    let tmp = TempDir::new().unwrap();
    let config = CostConfig {
        enabled: true,
        daily_limit_usd: 0.01, // Very low limit
        ..Default::default()
    };

    let tracker = CostTracker::new(config, tmp.path()).unwrap();

    // Record a usage that exceeds the limit
    let usage = TokenUsage::new("test/model", 10000, 5000, 1.0, 2.0); // ~0.02 USD
    tracker.record_usage(usage).unwrap();

    let check = tracker.check_budget(0.01).unwrap();
    assert!(matches!(check, BudgetCheck::Exceeded { .. }));
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
fn invalid_budget_estimate_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();

    let err = tracker.check_budget(f64::NAN).unwrap_err();
    assert!(err
        .to_string()
        .contains("Estimated cost must be a finite, non-negative value"));
}

#[test]
fn invalid_budget_negative_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    assert!(tracker.check_budget(-1.0).is_err());
}

#[test]
fn invalid_budget_infinity_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let tracker = CostTracker::new(enabled_config(), tmp.path()).unwrap();
    assert!(tracker.check_budget(f64::INFINITY).is_err());
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
fn budget_warning_threshold() {
    let tmp = TempDir::new().unwrap();
    let config = CostConfig {
        enabled: true,
        daily_limit_usd: 10.0,
        warn_at_percent: 80,
        monthly_limit_usd: 1000.0,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();

    // Record usage just under warning threshold (80% of 10 = 8.0)
    let _usage = TokenUsage::new("test/model", 100000, 50000, 1.0, 2.0);
    // This has a cost, so let's just check the budget with a projected amount
    let check = tracker.check_budget(8.5).unwrap();
    assert!(
        matches!(check, BudgetCheck::Warning { .. }),
        "expected warning, got {check:?}"
    );
}

#[test]
fn budget_monthly_exceeded() {
    let tmp = TempDir::new().unwrap();
    let config = CostConfig {
        enabled: true,
        daily_limit_usd: 1000.0,
        monthly_limit_usd: 0.01,
        ..Default::default()
    };
    let tracker = CostTracker::new(config, tmp.path()).unwrap();

    let usage = TokenUsage::new("test/model", 10000, 5000, 1.0, 2.0);
    tracker.record_usage(usage).unwrap();

    let check = tracker.check_budget(0.01).unwrap();
    assert!(matches!(
        check,
        BudgetCheck::Exceeded {
            period: UsagePeriod::Month,
            ..
        }
    ));
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
