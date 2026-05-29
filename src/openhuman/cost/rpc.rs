//! RPC handlers for the cost dashboard surface.
//!
//! The handlers prefer the process-global [`CostTracker`] populated at boot
//! by [`crate::openhuman::cost::init_global`]. When the global is missing —
//! e.g. when the dashboard RPC fires before bootstrap completes, or after a
//! tracker-construction failure — the handler constructs a fallback tracker
//! against the config-provided workspace so the UI gets an answer rather
//! than an error. The fallback is read-only by design: it shares the same
//! JSONL file as the real tracker and will see whatever is on disk.

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::openhuman::config::{Config, CostConfig};
use crate::rpc::RpcOutcome;

use super::global::try_global;
use super::tracker::CostTracker;
use super::types::{BudgetStatus, CostDashboard, CostSummary, DailyCostEntry, ModelStats};

#[derive(Debug, Clone, Serialize)]
pub struct DailyCostEntryDto {
    pub date: String,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub request_count: usize,
    pub by_model: Vec<ModelStatsDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatsDto {
    pub model: String,
    pub cost_usd: f64,
    pub total_tokens: u64,
    pub request_count: usize,
    pub provider: Option<String>,
    pub percent_of_total: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostDashboardDto {
    pub days: Vec<DailyCostEntryDto>,
    pub period_total_usd: f64,
    pub monthly_pace_usd: f64,
    pub budget_limit_monthly_usd: f64,
    pub month_to_date_usd: f64,
    pub budget_utilization: f64,
    pub budget_status: BudgetStatus,
    pub currency: String,
    pub warn_threshold: f64,
    pub alert_threshold: f64,
    pub enabled: bool,
    pub by_model: Vec<ModelStatsDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostSummaryDto {
    pub session_cost_usd: f64,
    pub daily_cost_usd: f64,
    pub monthly_cost_usd: f64,
    pub total_tokens: u64,
    pub request_count: usize,
    pub by_model: Vec<ModelStatsDto>,
}

fn provider_for(model: &str) -> Option<String> {
    model.split_once('/').map(|(prov, _)| prov.to_string())
}

fn model_stats_to_dto(stats: &ModelStats, total_cost: f64) -> ModelStatsDto {
    let percent_of_total = if total_cost > 0.0 {
        (stats.cost_usd / total_cost) * 100.0
    } else {
        0.0
    };
    ModelStatsDto {
        model: stats.model.clone(),
        cost_usd: stats.cost_usd,
        total_tokens: stats.total_tokens,
        request_count: stats.request_count,
        provider: provider_for(&stats.model),
        percent_of_total,
    }
}

fn daily_entry_to_dto(entry: &DailyCostEntry) -> DailyCostEntryDto {
    let mut by_model: Vec<ModelStatsDto> = entry
        .by_model
        .values()
        .map(|m| model_stats_to_dto(m, entry.cost_usd))
        .collect();
    by_model.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    DailyCostEntryDto {
        date: entry.date.format("%Y-%m-%d").to_string(),
        cost_usd: entry.cost_usd,
        input_tokens: entry.input_tokens,
        output_tokens: entry.output_tokens,
        total_tokens: entry.total_tokens,
        request_count: entry.request_count,
        by_model,
    }
}

fn dashboard_to_dto(dash: CostDashboard, cost_cfg: &CostConfig) -> CostDashboardDto {
    let total = dash.period_total_usd;
    let days = dash.days.iter().map(daily_entry_to_dto).collect();
    let by_model = dash
        .by_model
        .iter()
        .map(|m| model_stats_to_dto(m, total))
        .collect();
    CostDashboardDto {
        days,
        period_total_usd: dash.period_total_usd,
        monthly_pace_usd: dash.monthly_pace_usd,
        budget_limit_monthly_usd: dash.budget_limit_monthly_usd,
        month_to_date_usd: dash.month_to_date_usd,
        budget_utilization: dash.budget_utilization,
        budget_status: dash.budget_status,
        currency: dash.currency,
        warn_threshold: cost_cfg.dashboard.warn_threshold,
        alert_threshold: cost_cfg.dashboard.alert_threshold,
        enabled: cost_cfg.dashboard.enabled,
        by_model,
    }
}

fn summary_to_dto(s: &CostSummary) -> CostSummaryDto {
    let total = s.session_cost_usd;
    let mut by_model: Vec<ModelStatsDto> = s
        .by_model
        .values()
        .map(|m| model_stats_to_dto(m, total))
        .collect();
    by_model.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    CostSummaryDto {
        session_cost_usd: s.session_cost_usd,
        daily_cost_usd: s.daily_cost_usd,
        monthly_cost_usd: s.monthly_cost_usd,
        total_tokens: s.total_tokens,
        request_count: s.request_count,
        by_model,
    }
}

/// Cached fallback tracker used when the process-global tracker is
/// unavailable. Keyed on workspace path so a workspace switch rebuilds
/// the tracker rather than serving stale data. The cache also remembers
/// the last construction error and its timestamp, so repeated RPC polls
/// (every 10s from the UI hook) do not re-attempt a failing
/// `CostTracker::new` against the same bad workspace every call — the
/// failure is replayed for `FALLBACK_ERROR_TTL` before the next retry.
struct FallbackState {
    workspace: PathBuf,
    tracker: Option<Arc<CostTracker>>,
    last_error: Option<(Instant, String)>,
}

static FALLBACK_TRACKER: Mutex<Option<FallbackState>> = Mutex::new(None);
const FALLBACK_ERROR_TTL: Duration = Duration::from_secs(30);

fn resolve_tracker(config: &Config) -> Result<Arc<CostTracker>> {
    log::debug!(target: "cost_rpc", "[cost_rpc] resolve_tracker.start");
    if let Some(global) = try_global() {
        log::debug!(target: "cost_rpc", "[cost_rpc] resolve_tracker.global_hit");
        return Ok(global);
    }
    log::warn!(target: "cost_rpc", "[cost_rpc] resolve_tracker.global_miss — falling back to per-call tracker");

    let workspace = config.workspace_dir.clone();
    let mut guard = FALLBACK_TRACKER.lock();

    // Reuse the cached tracker only when the workspace path is unchanged.
    if let Some(state) = guard.as_ref() {
        if state.workspace == workspace {
            if let Some(tracker) = &state.tracker {
                log::debug!(target: "cost_rpc", "[cost_rpc] resolve_tracker.fallback_cached_hit");
                return Ok(tracker.clone());
            }
            if let Some((when, err)) = &state.last_error {
                if when.elapsed() < FALLBACK_ERROR_TTL {
                    log::debug!(
                        target: "cost_rpc",
                        "[cost_rpc] resolve_tracker.fallback_cached_error replay — err={err}"
                    );
                    return Err(anyhow!(
                        "cost tracker unavailable (cached failure, retry in {:?}): {err}",
                        FALLBACK_ERROR_TTL - when.elapsed()
                    ));
                }
            }
        }
    }

    match CostTracker::new(config.cost.clone(), &workspace) {
        Ok(tracker) => {
            log::debug!(target: "cost_rpc", "[cost_rpc] resolve_tracker.fallback_ready workspace={}", workspace.display());
            let arc = Arc::new(tracker);
            *guard = Some(FallbackState {
                workspace,
                tracker: Some(arc.clone()),
                last_error: None,
            });
            Ok(arc)
        }
        Err(err) => {
            let msg = format!("{err:#}");
            log::warn!(
                target: "cost_rpc",
                "[cost_rpc] resolve_tracker.fallback_failed workspace={} err={msg}",
                workspace.display()
            );
            *guard = Some(FallbackState {
                workspace,
                tracker: None,
                last_error: Some((Instant::now(), msg)),
            });
            Err(err).context("Failed to construct fallback CostTracker for dashboard RPC")
        }
    }
}

/// Build the dashboard payload for the current config.
pub fn dashboard(config: &Config) -> Result<RpcOutcome<Value>> {
    log::debug!(target: "cost_rpc", "[cost_rpc] dashboard.entry");
    let tracker = resolve_tracker(config).inspect_err(|err| {
        log::warn!(target: "cost_rpc", "[cost_rpc] dashboard.resolve_failed err={err:#}");
    })?;
    let dash = tracker
        .get_dashboard(
            &config.cost.dashboard.currency,
            config.cost.dashboard.warn_threshold,
            config.cost.dashboard.alert_threshold,
        )
        .inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc] dashboard.query_failed err={err:#}");
        })
        .context("cost dashboard query failed")?;
    let day_count = dash.days.len();
    let model_count = dash.by_model.len();
    let dto = dashboard_to_dto(dash, &config.cost);
    let value = serde_json::to_value(dto).context("cost dashboard serialize failed")?;
    log::debug!(
        target: "cost_rpc",
        "[cost_rpc] dashboard.exit days={day_count} models={model_count}"
    );
    Ok(RpcOutcome::new(value, Vec::new()))
}

/// Return the per-day cost history for the requested span.
pub fn daily_history(config: &Config, days: u32) -> Result<RpcOutcome<Value>> {
    log::debug!(target: "cost_rpc", "[cost_rpc] daily_history.entry days={days}");
    let tracker = resolve_tracker(config).inspect_err(|err| {
        log::warn!(target: "cost_rpc", "[cost_rpc] daily_history.resolve_failed err={err:#}");
    })?;
    let entries = tracker
        .get_daily_history(days)
        .inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc] daily_history.query_failed err={err:#}");
        })
        .context("cost daily history query failed")?;
    let entry_count = entries.len();
    let dto: Vec<DailyCostEntryDto> = entries.iter().map(daily_entry_to_dto).collect();
    let value = serde_json::to_value(dto).context("cost daily history serialize failed")?;
    log::debug!(target: "cost_rpc", "[cost_rpc] daily_history.exit entries={entry_count}");
    Ok(RpcOutcome::new(value, Vec::new()))
}

/// Return the live session / daily / monthly summary.
pub fn summary(config: &Config) -> Result<RpcOutcome<Value>> {
    log::debug!(target: "cost_rpc", "[cost_rpc] summary.entry");
    let tracker = resolve_tracker(config).inspect_err(|err| {
        log::warn!(target: "cost_rpc", "[cost_rpc] summary.resolve_failed err={err:#}");
    })?;
    let s = tracker
        .get_summary()
        .inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc] summary.query_failed err={err:#}");
        })
        .context("cost summary query failed")?;
    let request_count = s.request_count;
    let dto = summary_to_dto(&s);
    let value = serde_json::to_value(dto).context("cost summary serialize failed")?;
    log::debug!(target: "cost_rpc", "[cost_rpc] summary.exit requests={request_count}");
    Ok(RpcOutcome::new(value, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::cost::types::TokenUsage;
    use chrono::Utc;
    use std::collections::HashMap;
    use tempfile::TempDir;

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
    fn dashboard_rpc_returns_value_against_tempdir_workspace() {
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
        *FALLBACK_TRACKER.lock() = None;
        let (_tmp, cfg) = tempdir_config();
        let outcome = daily_history(&cfg, 0).expect("clamped to 1");
        let arr = outcome.value.as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn summary_rpc_returns_object() {
        *FALLBACK_TRACKER.lock() = None;
        let (_tmp, cfg) = tempdir_config();
        let outcome = summary(&cfg).expect("summary should resolve");
        let obj = outcome.value.as_object().unwrap();
        assert!(obj.contains_key("session_cost_usd"));
        assert!(obj.contains_key("by_model"));
    }

    #[test]
    fn resolve_tracker_caches_fallback_across_calls() {
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
}
