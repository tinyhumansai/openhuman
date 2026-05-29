use super::types::{
    BudgetCheck, BudgetStatus, CostDashboard, CostRecord, CostSummary, DailyCostEntry, ModelStats,
    TokenUsage, UsagePeriod,
};
use crate::openhuman::config::CostConfig;
use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use parking_lot::{Mutex, MutexGuard};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Cost tracker for API usage monitoring and budget enforcement.
pub struct CostTracker {
    config: CostConfig,
    storage: Arc<Mutex<CostStorage>>,
    session_id: String,
    session_costs: Arc<Mutex<Vec<CostRecord>>>,
}

impl CostTracker {
    /// Create a new cost tracker.
    pub fn new(config: CostConfig, workspace_dir: &Path) -> Result<Self> {
        let storage_path = resolve_storage_path(workspace_dir)?;

        let storage = CostStorage::new(&storage_path).with_context(|| {
            format!("Failed to open cost storage at {}", storage_path.display())
        })?;

        Ok(Self {
            config,
            storage: Arc::new(Mutex::new(storage)),
            session_id: uuid::Uuid::new_v4().to_string(),
            session_costs: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    fn lock_storage(&self) -> MutexGuard<'_, CostStorage> {
        self.storage.lock()
    }

    fn lock_session_costs(&self) -> MutexGuard<'_, Vec<CostRecord>> {
        self.session_costs.lock()
    }

    /// Check if a request is within budget.
    pub fn check_budget(&self, estimated_cost_usd: f64) -> Result<BudgetCheck> {
        if !self.config.enabled {
            return Ok(BudgetCheck::Allowed);
        }

        if !estimated_cost_usd.is_finite() || estimated_cost_usd < 0.0 {
            return Err(anyhow!(
                "Estimated cost must be a finite, non-negative value"
            ));
        }

        let mut storage = self.lock_storage();
        let (daily_cost, monthly_cost) = storage.get_aggregated_costs()?;

        // Check daily limit
        let projected_daily = daily_cost + estimated_cost_usd;
        if projected_daily > self.config.daily_limit_usd {
            return Ok(BudgetCheck::Exceeded {
                current_usd: daily_cost,
                limit_usd: self.config.daily_limit_usd,
                period: UsagePeriod::Day,
            });
        }

        // Check monthly limit
        let projected_monthly = monthly_cost + estimated_cost_usd;
        if projected_monthly > self.config.monthly_limit_usd {
            return Ok(BudgetCheck::Exceeded {
                current_usd: monthly_cost,
                limit_usd: self.config.monthly_limit_usd,
                period: UsagePeriod::Month,
            });
        }

        // Check warning thresholds
        let warn_threshold = f64::from(self.config.warn_at_percent.min(100)) / 100.0;
        let daily_warn_threshold = self.config.daily_limit_usd * warn_threshold;
        let monthly_warn_threshold = self.config.monthly_limit_usd * warn_threshold;

        if projected_daily >= daily_warn_threshold {
            return Ok(BudgetCheck::Warning {
                current_usd: daily_cost,
                limit_usd: self.config.daily_limit_usd,
                period: UsagePeriod::Day,
            });
        }

        if projected_monthly >= monthly_warn_threshold {
            return Ok(BudgetCheck::Warning {
                current_usd: monthly_cost,
                limit_usd: self.config.monthly_limit_usd,
                period: UsagePeriod::Month,
            });
        }

        Ok(BudgetCheck::Allowed)
    }

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
        let session_cost: f64 = session_costs
            .iter()
            .map(|record| record.usage.cost_usd)
            .sum();
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

    /// Get the monthly cost for a specific month.
    pub fn get_monthly_cost(&self, year: i32, month: u32) -> Result<f64> {
        let storage = self.lock_storage();
        storage.get_cost_for_month(year, month)
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
                });
            model_entry.cost_usd += record.usage.cost_usd;
            model_entry.total_tokens = model_entry
                .total_tokens
                .saturating_add(record.usage.total_tokens);
            model_entry.request_count += 1;
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
        let period_total_usd: f64 = days.iter().map(|d| d.cost_usd).sum();
        let daily_average = period_total_usd / days.len().max(1) as f64;
        let monthly_pace_usd = daily_average * 30.0;
        let budget_limit_monthly_usd = self.config.monthly_limit_usd.max(0.0);

        let now = Utc::now();
        let month_to_date_usd = self.get_monthly_cost(now.year(), now.month())?;
        let budget_utilization = if budget_limit_monthly_usd > 0.0 {
            (month_to_date_usd / budget_limit_monthly_usd).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let budget_status = if budget_limit_monthly_usd <= 0.0 {
            BudgetStatus::Normal
        } else {
            let warn = warn_threshold.clamp(0.0, 1.0);
            let alert = alert_threshold.clamp(0.0, 1.0).max(warn);
            let utilization_raw = month_to_date_usd / budget_limit_monthly_usd;
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
                    });
                entry.cost_usd += stats.cost_usd;
                entry.total_tokens = entry.total_tokens.saturating_add(stats.total_tokens);
                entry.request_count += stats.request_count;
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
            });

        entry.cost_usd += record.usage.cost_usd;
        entry.total_tokens += record.usage.total_tokens;
        entry.request_count += 1;
    }

    by_model
}

/// Persistent storage for cost records.
struct CostStorage {
    path: PathBuf,
    daily_cost_usd: f64,
    monthly_cost_usd: f64,
    cached_day: NaiveDate,
    cached_year: i32,
    cached_month: u32,
}

impl CostStorage {
    /// Create or open cost storage.
    fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }

        let now = Utc::now();
        let mut storage = Self {
            path: path.to_path_buf(),
            daily_cost_usd: 0.0,
            monthly_cost_usd: 0.0,
            cached_day: now.date_naive(),
            cached_year: now.year(),
            cached_month: now.month(),
        };

        storage.rebuild_aggregates(
            storage.cached_day,
            storage.cached_year,
            storage.cached_month,
        )?;

        Ok(storage)
    }

    fn for_each_record<F>(&self, mut on_record: F) -> Result<()>
    where
        F: FnMut(CostRecord),
    {
        if !self.path.exists() {
            return Ok(());
        }

        let file = File::open(&self.path)
            .with_context(|| format!("Failed to read cost storage from {}", self.path.display()))?;
        let reader = BufReader::new(file);

        for (line_number, line) in reader.lines().enumerate() {
            let raw_line = line.with_context(|| {
                format!(
                    "Failed to read line {} from cost storage {}",
                    line_number + 1,
                    self.path.display()
                )
            })?;

            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                continue;
            }

            match serde_json::from_str::<CostRecord>(trimmed) {
                Ok(record) => on_record(record),
                Err(error) => {
                    tracing::warn!(
                        "Skipping malformed cost record at {}:{}: {error}",
                        self.path.display(),
                        line_number + 1
                    );
                }
            }
        }

        Ok(())
    }

    fn rebuild_aggregates(&mut self, day: NaiveDate, year: i32, month: u32) -> Result<()> {
        let mut daily_cost = 0.0;
        let mut monthly_cost = 0.0;

        self.for_each_record(|record| {
            let timestamp = record.usage.timestamp.naive_utc();

            if timestamp.date() == day {
                daily_cost += record.usage.cost_usd;
            }

            if timestamp.year() == year && timestamp.month() == month {
                monthly_cost += record.usage.cost_usd;
            }
        })?;

        self.daily_cost_usd = daily_cost;
        self.monthly_cost_usd = monthly_cost;
        self.cached_day = day;
        self.cached_year = year;
        self.cached_month = month;

        Ok(())
    }

    fn ensure_period_cache_current(&mut self) -> Result<()> {
        let now = Utc::now();
        let day = now.date_naive();
        let year = now.year();
        let month = now.month();

        if day != self.cached_day || year != self.cached_year || month != self.cached_month {
            self.rebuild_aggregates(day, year, month)?;
        }

        Ok(())
    }

    /// Add a new record.
    fn add_record(&mut self, record: CostRecord) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("Failed to open cost storage at {}", self.path.display()))?;

        writeln!(file, "{}", serde_json::to_string(&record)?)
            .with_context(|| format!("Failed to write cost record to {}", self.path.display()))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync cost storage at {}", self.path.display()))?;

        self.ensure_period_cache_current()?;

        let timestamp = record.usage.timestamp.naive_utc();
        if timestamp.date() == self.cached_day {
            self.daily_cost_usd += record.usage.cost_usd;
        }
        if timestamp.year() == self.cached_year && timestamp.month() == self.cached_month {
            self.monthly_cost_usd += record.usage.cost_usd;
        }

        Ok(())
    }

    /// Get aggregated costs for current day and month.
    fn get_aggregated_costs(&mut self) -> Result<(f64, f64)> {
        self.ensure_period_cache_current()?;
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
}

#[cfg(test)]
#[path = "tracker_tests.rs"]
mod tests;
