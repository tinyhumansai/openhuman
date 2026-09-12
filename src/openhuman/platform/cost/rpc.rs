//! RPC handlers for the cost dashboard surface.
//!
//! The handlers prefer the process-global [`CostTracker`] populated at boot
//! by [`crate::openhuman::platform::cost::init_global`]. When the global is missing —
//! e.g. when the dashboard RPC fires before bootstrap completes, or after a
//! tracker-construction failure — the handler constructs a fallback tracker
//! against the config-provided workspace so the UI gets an answer rather
//! than an error. The fallback is read-only by design: it shares the same
//! JSONL file as the real tracker and will see whatever is on disk.

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::openhuman::config::{Config, CostConfig};
use crate::rpc::RpcOutcome;

use super::global::try_global;
use super::tracker::CostTracker;
use super::types::{
    BudgetStatus, CostDashboard, CostRecord, CostSource, CostSummary, DailyCostEntry, ModelStats,
};

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

#[derive(Debug, Clone, Serialize)]
pub struct UsageLogRecordDto {
    pub id: String,
    pub timestamp: String,
    pub session_id: String,
    pub model: String,
    pub provider: Option<String>,
    pub category: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
    pub cost_source: CostSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryStatsDto {
    pub category: String,
    pub cost_usd: f64,
    pub total_tokens: u64,
    pub request_count: usize,
    pub percent_of_total: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageLogDto {
    pub records: Vec<UsageLogRecordDto>,
    pub by_category: Vec<CategoryStatsDto>,
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub request_count: usize,
    pub currency: String,
    pub days: u32,
    pub limit: usize,
}

fn provider_for(model: &str) -> Option<String> {
    model.split_once('/').map(|(prov, _)| prov.to_string())
}

fn category_for(model: &str) -> String {
    let lower = model.to_lowercase();
    if lower.contains("embed") || lower.contains("text-embedding") || lower.contains("voyage") {
        "Embeddings".to_string()
    } else if lower.contains("whisper")
        || lower.contains("tts")
        || lower.contains("stt")
        || lower.contains("voice")
        || lower.contains("audio")
        || lower.contains("nova-")
    {
        "Voice and audio".to_string()
    } else if lower.contains("image")
        || lower.contains("dall-e")
        || lower.contains("gpt-image")
        || lower.contains("flux")
        || lower.contains("sdxl")
    {
        "Image generation".to_string()
    } else if lower.contains("rerank") {
        "Reranking".to_string()
    } else {
        "AI chat and reasoning".to_string()
    }
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

fn usage_record_to_dto(record: &CostRecord) -> UsageLogRecordDto {
    UsageLogRecordDto {
        id: record.id.clone(),
        timestamp: record.usage.timestamp.to_rfc3339(),
        session_id: record.session_id.clone(),
        model: record.usage.model.clone(),
        provider: provider_for(&record.usage.model),
        category: category_for(&record.usage.model),
        input_tokens: record.usage.input_tokens,
        output_tokens: record.usage.output_tokens,
        total_tokens: record.usage.total_tokens,
        cached_input_tokens: record.usage.cached_input_tokens,
        cache_creation_tokens: record.usage.cache_creation_tokens,
        reasoning_tokens: record.usage.reasoning_tokens,
        cost_usd: record.usage.cost_usd,
        cost_source: record.usage.cost_source,
    }
}

fn usage_log_to_dto(
    records: Vec<CostRecord>,
    currency: String,
    days: u32,
    limit: usize,
) -> UsageLogDto {
    let total_cost_usd: f64 = records.iter().map(|record| record.usage.cost_usd).sum();
    let total_tokens: u64 = records.iter().map(|record| record.usage.total_tokens).sum();
    let request_count = records.len();
    let mut by_category: HashMap<String, CategoryStatsDto> = HashMap::new();

    for record in &records {
        let category = category_for(&record.usage.model);
        let entry = by_category
            .entry(category.clone())
            .or_insert_with(|| CategoryStatsDto {
                category,
                cost_usd: 0.0,
                total_tokens: 0,
                request_count: 0,
                percent_of_total: 0.0,
            });
        entry.cost_usd += record.usage.cost_usd;
        entry.total_tokens = entry.total_tokens.saturating_add(record.usage.total_tokens);
        entry.request_count += 1;
    }

    let mut by_category: Vec<CategoryStatsDto> = by_category.into_values().collect();
    for category in &mut by_category {
        category.percent_of_total = if total_cost_usd > 0.0 {
            (category.cost_usd / total_cost_usd) * 100.0
        } else {
            0.0
        };
    }
    by_category.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.category.cmp(&b.category))
    });

    UsageLogDto {
        records: records.iter().map(usage_record_to_dto).collect(),
        by_category,
        total_cost_usd,
        total_tokens,
        request_count,
        currency,
        days,
        limit,
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

/// Return a recent, bounded usage log plus spend distribution by category.
pub fn usage_log(config: &Config, days: u32, limit: usize) -> Result<RpcOutcome<Value>> {
    log::debug!(target: "cost_rpc", "[cost_rpc] usage_log.entry days={days} limit={limit}");
    let tracker = resolve_tracker(config).inspect_err(|err| {
        log::warn!(target: "cost_rpc", "[cost_rpc] usage_log.resolve_failed err={err:#}");
    })?;
    let clamped_days = days.clamp(1, 366);
    let clamped_limit = limit.clamp(1, 1000);
    let records = tracker
        .get_recent_records(clamped_days, clamped_limit)
        .inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc] usage_log.query_failed err={err:#}");
        })
        .context("cost usage log query failed")?;
    let request_count = records.len();
    let dto = usage_log_to_dto(
        records,
        config.cost.dashboard.currency.clone(),
        clamped_days,
        clamped_limit,
    );
    let value = serde_json::to_value(dto).context("cost usage log serialize failed")?;
    log::debug!(target: "cost_rpc", "[cost_rpc] usage_log.exit records={request_count}");
    Ok(RpcOutcome::new(value, Vec::new()))
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
