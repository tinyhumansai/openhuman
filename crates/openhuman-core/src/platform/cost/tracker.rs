use super::route::route_for_model;
use super::types::{
    BudgetStatus, CostDashboard, CostRecord, CostSource, CostSummary, DailyCostEntry, ModelStats,
    TokenUsage,
};
use crate::config::CostConfig;
use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use parking_lot::{Mutex, MutexGuard};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Cost tracker for API usage monitoring and budget enforcement.
pub struct CostTracker {
    config: CostConfig,
    workspace_dir: PathBuf,
    storage: Arc<Mutex<CostStorage>>,
    session_id: String,
    session_costs: Arc<Mutex<Vec<CostRecord>>>,
}

impl CostTracker {
    /// Create a new cost tracker.
    pub fn new(config: CostConfig, workspace_dir: &Path) -> Result<Self> {
        let storage_path = resolve_storage_path(workspace_dir)?;

        let storage = CostStorage::new(workspace_dir, &storage_path);

        Ok(Self {
            config,
            workspace_dir: workspace_dir.to_path_buf(),
            storage: Arc::new(Mutex::new(storage)),
            session_id: uuid::Uuid::new_v4().to_string(),
            session_costs: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Workspace this tracker persists into.
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    fn lock_storage(&self) -> MutexGuard<'_, CostStorage> {
        self.storage.lock()
    }

    fn lock_session_costs(&self) -> MutexGuard<'_, Vec<CostRecord>> {
        self.session_costs.lock()
    }

    // `check_budget` and `BudgetCheck` were removed with the spend cap. Nothing
    // in the core refuses a request on cost any more: managed-credit
    // exhaustion is enforced server-side by the backend, which returns its own
    // billing error, and that is the only spend limit a user can now hit.
    // Recording below is untouched — the dashboard still sees every cost.

    /// Record a usage event.
    ///
    /// Honours `cost.enabled` — when budget enforcement is disabled the call
    /// is a no-op. Use [`Self::record_usage_unconditional`] for telemetry
    /// paths (dashboard, observability) that must capture data even when
    /// the budget enforcement path is off, so the user can flip enforcement
    /// on later without losing history.
    pub fn record_usage(&self, usage: TokenUsage) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        self.record_usage_unconditional(usage)
    }

    /// Persist a usage event ignoring `cost.enabled`. Used by the dashboard
    /// telemetry hook so cost history is recorded regardless of whether the
    /// budget-enforcement gate is on.
    pub fn record_usage_unconditional(&self, usage: TokenUsage) -> Result<()> {
        if !usage.cost_usd.is_finite() || usage.cost_usd < 0.0 {
            return Err(anyhow!(
                "Token usage cost must be a finite, non-negative value"
            ));
        }

        let record = CostRecord::new(&self.session_id, usage);

        // Persist first for durability guarantees.
        {
            let mut storage = self.lock_storage();
            storage.add_record(record.clone())?;
        }

        // Then update in-memory session snapshot.
        let mut session_costs = self.lock_session_costs();
        session_costs.push(record);

        Ok(())
    }

    /// Get the current cost summary.
    pub fn get_summary(&self) -> Result<CostSummary> {
        let (daily_cost, monthly_cost) = {
            let mut storage = self.lock_storage();
            storage.get_aggregated_costs()?
        };

        let session_costs = self.lock_session_costs();
        let session_cost: f64 = non_negative_zero(
            session_costs
                .iter()
                .map(|record| record.usage.cost_usd)
                .sum(),
        );
        let total_tokens: u64 = session_costs
            .iter()
            .map(|record| record.usage.total_tokens)
            .sum();
        let request_count = session_costs.len();
        let by_model = build_session_model_stats(&session_costs);

        Ok(CostSummary {
            session_cost_usd: session_cost,
            daily_cost_usd: daily_cost,
            monthly_cost_usd: monthly_cost,
            total_tokens,
            request_count,
            by_model,
        })
    }

    /// Get the daily cost for a specific date.
    pub fn get_daily_cost(&self, date: NaiveDate) -> Result<f64> {
        let storage = self.lock_storage();
        storage.get_cost_for_date(date)
    }

    /// Get the monthly cost for a specific month, across all routes.
    pub fn get_monthly_cost(&self, year: i32, month: u32) -> Result<f64> {
        let storage = self.lock_storage();
        storage.get_cost_for_month(year, month)
    }

    /// Get the **managed-route** monthly cost for a specific month — the
    /// portion of spend the local `[cost]` budget applies to (#5016). BYOK and
    /// local inference is excluded because OpenHuman never bills for it.
    pub fn get_managed_monthly_cost(&self, year: i32, month: u32) -> Result<f64> {
        let storage = self.lock_storage();
        storage.get_managed_cost_for_month(year, month)
    }

    /// Get a daily cost/token history covering the last `days` calendar days,
    /// ending on today (UTC) inclusive. Days with no recorded usage are
    /// returned as zero-filled entries so callers can render the chart bars
    /// without gap handling. Oldest day first.
    ///
    /// `days` is clamped to the range `[1, 366]` to bound the scan window.
    pub fn get_daily_history(&self, days: u32) -> Result<Vec<DailyCostEntry>> {
        let span = days.clamp(1, 366) as i64;
        let today = Utc::now().date_naive();
        let earliest = today
            .checked_sub_signed(Duration::days(span - 1))
            .ok_or_else(|| anyhow!("Daily history range underflowed"))?;

        let mut buckets: BTreeMap<NaiveDate, DailyCostEntry> = BTreeMap::new();
        let storage = self.lock_storage();
        storage.for_each_record(|record| {
            let date = record.usage.timestamp.naive_utc().date();
            if date < earliest || date > today {
                return;
            }

            let entry = buckets
                .entry(date)
                .or_insert_with(|| DailyCostEntry::empty(date));
            entry.cost_usd += record.usage.cost_usd;
            entry.input_tokens = entry.input_tokens.saturating_add(record.usage.input_tokens);
            entry.output_tokens = entry
                .output_tokens
                .saturating_add(record.usage.output_tokens);
            entry.total_tokens = entry.total_tokens.saturating_add(record.usage.total_tokens);
            entry.request_count += 1;

            let model_entry = entry
                .by_model
                .entry(record.usage.model.clone())
                .or_insert_with(|| ModelStats {
                    model: record.usage.model.clone(),
                    cost_usd: 0.0,
                    total_tokens: 0,
                    request_count: 0,
                    unpriced_request_count: 0,
                });
            model_entry.cost_usd += record.usage.cost_usd;
            model_entry.total_tokens = model_entry
                .total_tokens
                .saturating_add(record.usage.total_tokens);
            model_entry.request_count += 1;
            if record.usage.cost_source == CostSource::Unknown {
                model_entry.unpriced_request_count += 1;
            }
        })?;

        let mut out = Vec::with_capacity(span as usize);
        for offset in 0..span {
            let date = earliest + Duration::days(offset);
            out.push(
                buckets
                    .remove(&date)
                    .unwrap_or_else(|| DailyCostEntry::empty(date)),
            );
        }
        Ok(out)
    }

    /// Return recent persisted usage records, newest first.
    ///
    /// `days` is a rolling window ending now: `days = 1` is the last 24 hours,
    /// matching what the dashboard promises the user ("Newest records from the
    /// last {days} days").
    ///
    /// The window is `days * 24h` back from this instant, NOT `days - 1`.
    /// [`Self::get_daily_history`] subtracts `span - 1` and is right to: it
    /// compares *dates* and needs N calendar days counting today, so
    /// `today - (N - 1)` is the Nth day back. Here the comparison is against an
    /// *instant*, so the same `- 1` shortens the window by a full day — and at
    /// `days = 1` collapses it to zero, making `earliest == now` and rejecting
    /// every record ever written (#6482). Wider windows hid it: `days = 30`,
    /// the dashboard's default, quietly returned 29.
    ///
    /// `days` is clamped to `[1, 366]` and `limit` to `[1, 1000]` to keep
    /// dashboard calls bounded while still allowing a detailed audit log.
    pub fn get_recent_records(&self, days: u32, limit: usize) -> Result<Vec<CostRecord>> {
        let span = days.clamp(1, 366) as i64;
        let limit = limit.clamp(1, 1000);
        let now = Utc::now();
        let earliest = now
            .checked_sub_signed(Duration::days(span))
            .ok_or_else(|| anyhow!("Usage log range underflowed"))?;

        let mut records: Vec<CostRecord> = Vec::new();
        let storage = self.lock_storage();
        storage.for_each_record(|record| {
            if record.usage.timestamp < earliest || record.usage.timestamp > now {
                return;
            }
            records.push(record);
        })?;

        records.sort_by(|a, b| {
            b.usage
                .timestamp
                .cmp(&a.usage.timestamp)
                .then_with(|| b.id.cmp(&a.id))
        });
        records.truncate(limit);
        Ok(records)
    }

    /// Every record timestamped in `[from, to]`, oldest first — the input to
    /// [`super::report`]. The window is capped at 366 days, like the other
    /// reads, so one call never scans more history than the dashboard does.
    pub fn records_between(
        &self,
        from: chrono::DateTime<Utc>,
        to: chrono::DateTime<Utc>,
    ) -> Result<Vec<CostRecord>> {
        let floor = to
            .checked_sub_signed(Duration::days(366))
            .ok_or_else(|| anyhow!("Usage report range underflowed"))?;
        let from = from.max(floor);
        let mut records: Vec<CostRecord> = Vec::new();
        let storage = self.lock_storage();
        storage.for_each_record(|record| {
            if record.usage.timestamp >= from && record.usage.timestamp <= to {
                records.push(record);
            }
        })?;
        records.sort_by_key(|record| record.usage.timestamp);
        Ok(records)
    }

    /// Build the full dashboard payload: 7-day history, period total,
    /// projected monthly pace (daily avg × 30), and budget utilisation
    /// derived from the configured monthly limit and warn/alert thresholds.
    ///
    /// `warn_threshold` / `alert_threshold` are fractions of the monthly
    /// budget — e.g. 0.8 (warn at 80%) and 0.95 (alert at 95%). When the
    /// monthly limit is non-positive, status falls back to `Normal`.
    pub fn get_dashboard(
        &self,
        currency: &str,
        warn_threshold: f64,
        alert_threshold: f64,
    ) -> Result<CostDashboard> {
        let days = self.get_daily_history(7)?;
        let period_total_usd: f64 = non_negative_zero(days.iter().map(|d| d.cost_usd).sum());
        let daily_average = period_total_usd / days.len().max(1) as f64;
        let monthly_pace_usd = daily_average * 30.0;
        let budget_limit_monthly_usd = self.config.monthly_limit_usd.max(0.0);

        let now = Utc::now();
        // `month_to_date_usd` stays an all-route total: users want to see what
        // their BYOK providers cost them. Budget utilisation and status,
        // however, describe a limit that only gates *managed* spend (#5016) —
        // driving them off the total made a pure-BYOK user's gauge fill to
        // 100% against a cap that can never fire, which is the phantom limit
        // reported in the issue.
        let month_to_date_usd = self.get_monthly_cost(now.year(), now.month())?;
        let managed_month_to_date_usd = self.get_managed_monthly_cost(now.year(), now.month())?;
        let budget_utilization = if budget_limit_monthly_usd > 0.0 {
            (managed_month_to_date_usd / budget_limit_monthly_usd).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let budget_status = if budget_limit_monthly_usd <= 0.0 {
            BudgetStatus::Normal
        } else {
            let warn = warn_threshold.clamp(0.0, 1.0);
            let alert = alert_threshold.clamp(0.0, 1.0).max(warn);
            let utilization_raw = managed_month_to_date_usd / budget_limit_monthly_usd;
            if utilization_raw >= alert {
                BudgetStatus::Exceeded
            } else if utilization_raw >= warn {
                BudgetStatus::Warning
            } else {
                BudgetStatus::Normal
            }
        };

        let mut by_model_totals: HashMap<String, ModelStats> = HashMap::new();
        for day in &days {
            for (model, stats) in &day.by_model {
                let entry = by_model_totals
                    .entry(model.clone())
                    .or_insert_with(|| ModelStats {
                        model: model.clone(),
                        cost_usd: 0.0,
                        total_tokens: 0,
                        request_count: 0,
                        unpriced_request_count: 0,
                    });
                entry.cost_usd += stats.cost_usd;
                entry.total_tokens = entry.total_tokens.saturating_add(stats.total_tokens);
                entry.request_count += stats.request_count;
                entry.unpriced_request_count += stats.unpriced_request_count;
            }
        }
        let mut by_model: Vec<ModelStats> = by_model_totals.into_values().collect();
        by_model.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.model.cmp(&b.model))
        });

        Ok(CostDashboard {
            days,
            period_total_usd,
            monthly_pace_usd,
            budget_limit_monthly_usd,
            month_to_date_usd,
            budget_utilization,
            budget_status,
            currency: currency.to_string(),
            by_model,
        })
    }
}

/// An empty `f64` sum is `-0.0`, which serialises as `-0.0` and renders as
/// "-$0.00". Every derived total goes through this.
pub(super) fn non_negative_zero(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Rows the budget gate once wrote as `host:<agent_id>` with an estimated cost
/// of zero. The event bridge already recorded the same model call under its
/// real model, so these only double the token and request counts. Ledgers
/// written before that stopped still hold them; every read skips them.
fn is_legacy_host_duplicate(record: &CostRecord) -> bool {
    record.usage.model.starts_with("host:") && record.usage.cost_source == CostSource::Estimated
}

fn resolve_storage_path(workspace_dir: &Path) -> Result<PathBuf> {
    let storage_path = workspace_dir.join("state").join("costs.jsonl");
    let legacy_path = workspace_dir.join(".openhuman").join("costs.db");

    if !storage_path.exists() && legacy_path.exists() {
        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }

        if let Err(error) = fs::rename(&legacy_path, &storage_path) {
            tracing::warn!(
                "Failed to move legacy cost storage from {} to {}: {error}; falling back to copy",
                legacy_path.display(),
                storage_path.display()
            );
            fs::copy(&legacy_path, &storage_path).with_context(|| {
                format!(
                    "Failed to copy legacy cost storage from {} to {}",
                    legacy_path.display(),
                    storage_path.display()
                )
            })?;
        }
    }

    Ok(storage_path)
}

fn build_session_model_stats(session_costs: &[CostRecord]) -> HashMap<String, ModelStats> {
    let mut by_model: HashMap<String, ModelStats> = HashMap::new();

    for record in session_costs {
        let entry = by_model
            .entry(record.usage.model.clone())
            .or_insert_with(|| ModelStats {
                model: record.usage.model.clone(),
                cost_usd: 0.0,
                total_tokens: 0,
                request_count: 0,
                unpriced_request_count: 0,
            });

        entry.cost_usd += record.usage.cost_usd;
        entry.total_tokens += record.usage.total_tokens;
        entry.request_count += 1;
        if record.usage.cost_source == CostSource::Unknown {
            entry.unpriced_request_count += 1;
        }
    }

    by_model
}

/// Persistent storage for cost records.
struct CostStorage {
    /// Workspace whose default backend serves the ledger when the host
    /// installed none.
    workspace_dir: PathBuf,
    /// The legacy JSONL ledger, imported into the document ledger once.
    path: PathBuf,
    daily_cost_usd: f64,
    monthly_cost_usd: f64,
    /// Managed-route subset of `daily_cost_usd` — the only spend the local
    /// `[cost]` budget may gate (#5016). BYOK/local spend is still counted in
    /// the totals above so the dashboard shows it, but never here.
    daily_managed_cost_usd: f64,
    /// Managed-route subset of `monthly_cost_usd`. See above.
    monthly_managed_cost_usd: f64,
}

impl CostStorage {
    /// Open the cost ledger. The records live in documents under the acting
    /// agent's scope, which is only known per call, so there is no directory
    /// to make and no aggregate to prime here.
    fn new(workspace_dir: &Path, path: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            path: path.to_path_buf(),
            daily_cost_usd: 0.0,
            monthly_cost_usd: 0.0,
            daily_managed_cost_usd: 0.0,
            monthly_managed_cost_usd: 0.0,
        }
    }

    fn for_each_record<F>(&self, mut on_record: F) -> Result<()>
    where
        F: FnMut(CostRecord),
    {
        let docs = super::tracker_documents::current(&self.workspace_dir)?;
        self.import_legacy_once(&docs)?;
        for record in docs.all()? {
            if !is_legacy_host_duplicate(&record) {
                on_record(record);
            }
        }
        Ok(())
    }

    /// The workspace's JSONL ledger predates the backend and belongs to the
    /// single-user (`local`) scope; import it into that scope once.
    /// Once per process for the backend instance (see
    /// `CostDocs::import_legacy_once`).
    fn import_legacy_once(&self, docs: &super::tracker_documents::CostDocs) -> Result<()> {
        if crate::storage::current_scope().ok() != Some(crate::storage::Scope::local()) {
            return Ok(());
        }
        docs.import_legacy_once(&self.path)
    }

    fn rebuild_aggregates(&mut self, day: NaiveDate, year: i32, month: u32) -> Result<()> {
        let mut daily_cost = 0.0;
        let mut monthly_cost = 0.0;
        let mut daily_managed_cost = 0.0;
        let mut monthly_managed_cost = 0.0;

        self.for_each_record(|record| {
            let timestamp = record.usage.timestamp.naive_utc();
            // Classified from the record's own model id, so records written by
            // builds that predate #5016 are routed correctly with no migration.
            let counts = route_for_model(&record.usage.model).counts_toward_budget();

            if timestamp.date() == day {
                daily_cost += record.usage.cost_usd;
                if counts {
                    daily_managed_cost += record.usage.cost_usd;
                }
            }

            if timestamp.year() == year && timestamp.month() == month {
                monthly_cost += record.usage.cost_usd;
                if counts {
                    monthly_managed_cost += record.usage.cost_usd;
                }
            }
        })?;

        self.daily_cost_usd = daily_cost;
        self.monthly_cost_usd = monthly_cost;
        self.daily_managed_cost_usd = daily_managed_cost;
        self.monthly_managed_cost_usd = monthly_managed_cost;

        Ok(())
    }

    /// Add a new record.
    fn add_record(&mut self, record: CostRecord) -> Result<()> {
        let docs = super::tracker_documents::current(&self.workspace_dir)?;
        // The legacy ledger goes in before the first document, so history
        // is never split across the two.
        self.import_legacy_once(&docs)?;
        docs.add(&record)
    }

    /// Get aggregated costs for current day and month, across **all** routes.
    /// This is the dashboard/summary figure — use
    /// [`Self::get_aggregated_managed_costs`] for anything that gates a
    /// request.
    fn get_aggregated_costs(&mut self) -> Result<(f64, f64)> {
        // The totals are recomputed from the acting scope's records on every
        // call: the scope changes per call, so a process-wide cache would leak
        // one agent's spend into another's.
        let now = Utc::now();
        self.rebuild_aggregates(now.date_naive(), now.year(), now.month())?;
        Ok((self.daily_cost_usd, self.monthly_cost_usd))
    }

    /// Get cost for a specific date.
    fn get_cost_for_date(&self, date: NaiveDate) -> Result<f64> {
        let mut cost = 0.0;

        self.for_each_record(|record| {
            if record.usage.timestamp.naive_utc().date() == date {
                cost += record.usage.cost_usd;
            }
        })?;

        Ok(cost)
    }

    /// Get cost for a specific month.
    fn get_cost_for_month(&self, year: i32, month: u32) -> Result<f64> {
        let mut cost = 0.0;

        self.for_each_record(|record| {
            let timestamp = record.usage.timestamp.naive_utc();
            if timestamp.year() == year && timestamp.month() == month {
                cost += record.usage.cost_usd;
            }
        })?;

        Ok(cost)
    }

    /// Get **managed-route** cost for a specific month — the figure the budget
    /// gauge should reflect, since only managed spend can trip the limit
    /// (#5016).
    fn get_managed_cost_for_month(&self, year: i32, month: u32) -> Result<f64> {
        let mut cost = 0.0;

        self.for_each_record(|record| {
            let timestamp = record.usage.timestamp.naive_utc();
            if timestamp.year() == year
                && timestamp.month() == month
                && route_for_model(&record.usage.model).counts_toward_budget()
            {
                cost += record.usage.cost_usd;
            }
        })?;

        Ok(cost)
    }
}

#[cfg(test)]
#[path = "tracker_tests.rs"]
mod tests;
