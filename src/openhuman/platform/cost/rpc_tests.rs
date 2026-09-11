use super::*;
use crate::openhuman::platform::cost::types::TokenUsage;
use chrono::Utc;
use std::collections::HashMap;
use tempfile::TempDir;

/// Serialize all tests that mutate the process-global `FALLBACK_TRACKER`
/// so they don't race each other within the same test binary.
fn tracker_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn tempdir_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.cost.enabled = true;
    cfg.cost.monthly_limit_usd = 100.0;
    cfg.cost.dashboard.warn_threshold = 0.8;
    cfg.cost.dashboard.alert_threshold = 0.95;
    cfg.cost.dashboard.currency = "USD".to_string();
    cfg.cost.dashboard.enabled = true;
    (tmp, cfg)
}

fn make_model_stats(model: &str, cost: f64) -> ModelStats {
    ModelStats {
        model: model.to_string(),
        cost_usd: cost,
        total_tokens: 1500,
        request_count: 1,
    }
}

#[test]
fn provider_for_extracts_namespace() {
    assert_eq!(
        provider_for("anthropic/claude-sonnet-4"),
        Some("anthropic".to_string())
    );
    assert_eq!(provider_for("openai/gpt-5"), Some("openai".to_string()));
    assert_eq!(provider_for("bare-model"), None);
}

#[test]
fn category_for_classifies_common_usage_families() {
    assert_eq!(category_for("voyage/voyage-3"), "Embeddings");
    assert_eq!(category_for("openai/whisper-1"), "Voice and audio");
    assert_eq!(category_for("openai/gpt-image-1"), "Image generation");
    assert_eq!(category_for("cohere/rerank-english"), "Reranking");
    assert_eq!(
        category_for("anthropic/claude-sonnet-4"),
        "AI chat and reasoning"
    );
}

#[test]
fn model_stats_dto_percent_zero_when_total_zero() {
    let stats = make_model_stats("a/b", 0.0);
    let dto = model_stats_to_dto(&stats, 0.0);
    assert_eq!(dto.percent_of_total, 0.0);
    assert_eq!(dto.provider.as_deref(), Some("a"));
}

#[test]
fn model_stats_dto_percent_scales_with_total() {
    let stats = make_model_stats("anthropic/x", 2.5);
    let dto = model_stats_to_dto(&stats, 10.0);
    assert!((dto.percent_of_total - 25.0).abs() < f64::EPSILON);
}

#[test]
fn daily_entry_dto_sorts_models_by_cost_desc_and_formats_date() {
    let mut by_model = HashMap::new();
    by_model.insert("a".to_string(), make_model_stats("a", 1.0));
    by_model.insert("b".to_string(), make_model_stats("b", 3.0));
    by_model.insert("c".to_string(), make_model_stats("c", 2.0));
    let entry = DailyCostEntry {
        date: chrono::NaiveDate::from_ymd_opt(2026, 5, 27).unwrap(),
        cost_usd: 6.0,
        input_tokens: 1000,
        output_tokens: 500,
        total_tokens: 1500,
        request_count: 3,
        by_model,
    };
    let dto = daily_entry_to_dto(&entry);
    assert_eq!(dto.date, "2026-05-27");
    assert_eq!(dto.by_model.len(), 3);
    assert_eq!(dto.by_model[0].model, "b");
    assert_eq!(dto.by_model[1].model, "c");
    assert_eq!(dto.by_model[2].model, "a");
}

#[test]
fn dashboard_dto_propagates_threshold_and_enabled_flags() {
    let (_tmp, cfg) = tempdir_config();
    let dash = CostDashboard {
        days: vec![],
        period_total_usd: 0.0,
        monthly_pace_usd: 0.0,
        budget_limit_monthly_usd: 100.0,
        month_to_date_usd: 0.0,
        budget_utilization: 0.0,
        budget_status: BudgetStatus::Normal,
        currency: "USD".to_string(),
        by_model: vec![],
    };
    let dto = dashboard_to_dto(dash, &cfg.cost);
    assert!((dto.warn_threshold - 0.8).abs() < f64::EPSILON);
    assert!((dto.alert_threshold - 0.95).abs() < f64::EPSILON);
    assert!(dto.enabled);
}

#[test]
fn summary_dto_sorts_models_by_cost_desc() {
    let mut by_model = HashMap::new();
    by_model.insert("low".to_string(), make_model_stats("low", 0.5));
    by_model.insert("high".to_string(), make_model_stats("high", 5.0));
    let summary = CostSummary {
        session_cost_usd: 5.5,
        daily_cost_usd: 5.5,
        monthly_cost_usd: 5.5,
        total_tokens: 3000,
        request_count: 2,
        by_model,
    };
    let dto = summary_to_dto(&summary);
    assert_eq!(dto.by_model.len(), 2);
    assert_eq!(dto.by_model[0].model, "high");
    assert_eq!(dto.by_model[1].model, "low");
}

#[test]
fn usage_log_dto_sorts_categories_and_preserves_records() {
    let mut chat = CostRecord::new(
        "session-a",
        TokenUsage::new("anthropic/claude-sonnet-4", 1000, 500, 0.0, 0.0),
    );
    chat.usage.cost_usd = 3.0;
    chat.usage.cached_input_tokens = 250;
    chat.usage.reasoning_tokens = 32;
    let mut embeddings = CostRecord::new(
        "session-b",
        TokenUsage::new("voyage/voyage-3", 2000, 0, 0.0, 0.0),
    );
    embeddings.usage.cost_usd = 1.0;

    let dto = usage_log_to_dto(vec![chat, embeddings], "USD".to_string(), 30, 100);
    assert_eq!(dto.records.len(), 2);
    assert_eq!(dto.by_category.len(), 2);
    assert_eq!(dto.by_category[0].category, "AI chat and reasoning");
    assert!((dto.by_category[0].percent_of_total - 75.0).abs() < f64::EPSILON);
    assert_eq!(dto.total_tokens, 3500);
    assert_eq!(dto.records[0].cached_input_tokens, 250);
    assert_eq!(dto.records[0].reasoning_tokens, 32);
    assert_eq!(dto.records[0].cost_source, CostSource::Estimated);
}

#[test]
fn dashboard_rpc_returns_value_against_tempdir_workspace() {
    let _lock = tracker_test_lock();
    // Reset FALLBACK_TRACKER state so a previous test's cache cannot
    // interfere with this isolated workspace.
    *FALLBACK_TRACKER.lock() = None;
    let (_tmp, cfg) = tempdir_config();
    let outcome = dashboard(&cfg).expect("dashboard should resolve");
    let payload = outcome.value;
    assert!(payload.is_object());
    let days = payload.get("days").and_then(|v| v.as_array()).unwrap();
    assert_eq!(days.len(), 7);
}

#[test]
fn daily_history_rpc_clamps_and_returns_array() {
    let _lock = tracker_test_lock();
    *FALLBACK_TRACKER.lock() = None;
    let (_tmp, cfg) = tempdir_config();
    let outcome = daily_history(&cfg, 0).expect("clamped to 1");
    let arr = outcome.value.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn summary_rpc_returns_object() {
    let _lock = tracker_test_lock();
    *FALLBACK_TRACKER.lock() = None;
    let (_tmp, cfg) = tempdir_config();
    let outcome = summary(&cfg).expect("summary should resolve");
    let obj = outcome.value.as_object().unwrap();
    assert!(obj.contains_key("session_cost_usd"));
    assert!(obj.contains_key("by_model"));
}

#[test]
fn usage_log_rpc_returns_records_and_category_breakdown() {
    let _lock = tracker_test_lock();
    if try_global().is_some() {
        return;
    }
    *FALLBACK_TRACKER.lock() = None;
    let (_tmp, cfg) = tempdir_config();
    let tracker = resolve_tracker(&cfg).unwrap();
    let mut usage = TokenUsage::new("anthropic/claude-sonnet-4", 1000, 500, 0.0, 0.0);
    usage.cost_usd = 1.25;
    usage.timestamp = Utc::now();
    tracker.record_usage_unconditional(usage).unwrap();

    let outcome = usage_log(&cfg, 30, 100).expect("usage log should resolve");
    let obj = outcome.value.as_object().unwrap();
    assert_eq!(obj["request_count"], 1);
    assert_eq!(obj["records"].as_array().unwrap().len(), 1);
    assert_eq!(obj["by_category"].as_array().unwrap().len(), 1);
}

#[test]
fn resolve_tracker_caches_fallback_across_calls() {
    let _lock = tracker_test_lock();
    *FALLBACK_TRACKER.lock() = None;
    let (_tmp, cfg) = tempdir_config();
    let first = resolve_tracker(&cfg).unwrap();
    let second = resolve_tracker(&cfg).unwrap();
    // Both calls return Arc<CostTracker>; when no global is set the
    // second call must hit the cached fallback (same Arc pointer).
    if try_global().is_none() {
        assert!(Arc::ptr_eq(&first, &second));
    }
}

#[test]
fn resolve_tracker_replays_cached_error_until_ttl() {
    let _lock = tracker_test_lock();
    // Pre-seed cache with a synthetic failure. Even though
    // CostTracker::new would succeed against this tempdir, the cache
    // takes precedence until the TTL elapses.
    let (_tmp, cfg) = tempdir_config();
    // Only meaningful when no global is set; otherwise try_global wins.
    if try_global().is_some() {
        return;
    }
    *FALLBACK_TRACKER.lock() = Some(FallbackState {
        workspace: cfg.workspace_dir.clone(),
        tracker: None,
        last_error: Some((Instant::now(), "synthetic".to_string())),
    });
    let err = match resolve_tracker(&cfg) {
        Ok(_) => panic!("expected cached failure replay"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("cached failure"), "got: {err}");
}

#[test]
fn dashboard_query_includes_persisted_record() {
    let _lock = tracker_test_lock();
    // Skip when the process-global tracker has been initialised by a
    // sibling test — the global is one-shot per process and points
    // at whatever workspace won the race, so we cannot reliably
    // round-trip a record through `cfg.workspace_dir` here.
    if try_global().is_some() {
        return;
    }
    *FALLBACK_TRACKER.lock() = None;
    let (_tmp, cfg) = tempdir_config();
    let tracker = resolve_tracker(&cfg).unwrap();
    let mut usage = TokenUsage::new("anthropic/claude-sonnet-4", 1000, 500, 0.0, 0.0);
    usage.cost_usd = 1.25;
    usage.timestamp = Utc::now();
    tracker.record_usage_unconditional(usage).unwrap();
    let outcome = dashboard(&cfg).expect("dashboard should resolve");
    let total = outcome
        .value
        .get("period_total_usd")
        .unwrap()
        .as_f64()
        .unwrap();
    assert!((1.24..=1.26).contains(&total), "got total {total}");
}
