use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Token usage information from a single API call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Model identifier (e.g., "anthropic/claude-sonnet-4-20250514")
    pub model: String,
    /// Input/prompt tokens
    pub input_tokens: u64,
    /// Output/completion tokens
    pub output_tokens: u64,
    /// Total tokens
    pub total_tokens: u64,
    /// Calculated cost in USD
    pub cost_usd: f64,
    /// Timestamp of the request
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl TokenUsage {
    fn sanitize_price(value: f64) -> f64 {
        if value.is_finite() && value > 0.0 {
            value
        } else {
            0.0
        }
    }

    /// Create a new token usage record.
    pub fn new(
        model: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        input_price_per_million: f64,
        output_price_per_million: f64,
    ) -> Self {
        let model = model.into();
        let input_price_per_million = Self::sanitize_price(input_price_per_million);
        let output_price_per_million = Self::sanitize_price(output_price_per_million);
        let total_tokens = input_tokens.saturating_add(output_tokens);

        // Calculate cost: (tokens / 1M) * price_per_million
        let input_cost = (input_tokens as f64 / 1_000_000.0) * input_price_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * output_price_per_million;
        let cost_usd = input_cost + output_cost;

        Self {
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost_usd,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Get the total cost.
    pub fn cost(&self) -> f64 {
        self.cost_usd
    }
}

/// Time period for cost aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsagePeriod {
    Session,
    Day,
    Month,
}

/// A single cost record for persistent storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    /// Unique identifier
    pub id: String,
    /// Token usage details
    pub usage: TokenUsage,
    /// Session identifier (for grouping)
    pub session_id: String,
}

impl CostRecord {
    /// Create a new cost record.
    pub fn new(session_id: impl Into<String>, usage: TokenUsage) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            usage,
            session_id: session_id.into(),
        }
    }
}

/// Budget enforcement result.
#[derive(Debug, Clone)]
pub enum BudgetCheck {
    /// Within budget, request can proceed
    Allowed,
    /// Warning threshold exceeded but request can proceed
    Warning {
        current_usd: f64,
        limit_usd: f64,
        period: UsagePeriod,
    },
    /// Budget exceeded, request blocked
    Exceeded {
        current_usd: f64,
        limit_usd: f64,
        period: UsagePeriod,
    },
}

/// Cost summary for reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSummary {
    /// Total cost for the session
    pub session_cost_usd: f64,
    /// Total cost for the day
    pub daily_cost_usd: f64,
    /// Total cost for the month
    pub monthly_cost_usd: f64,
    /// Total tokens used
    pub total_tokens: u64,
    /// Number of requests
    pub request_count: usize,
    /// Breakdown by model
    pub by_model: std::collections::HashMap<String, ModelStats>,
}

/// Statistics for a specific model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStats {
    /// Model name
    pub model: String,
    /// Total cost for this model
    pub cost_usd: f64,
    /// Total tokens for this model
    pub total_tokens: u64,
    /// Number of requests for this model
    pub request_count: usize,
}

impl Default for CostSummary {
    fn default() -> Self {
        Self {
            session_cost_usd: 0.0,
            daily_cost_usd: 0.0,
            monthly_cost_usd: 0.0,
            total_tokens: 0,
            request_count: 0,
            by_model: std::collections::HashMap::new(),
        }
    }
}

/// Per-day aggregate of cost and token usage for the dashboard charts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyCostEntry {
    /// Calendar date in UTC (YYYY-MM-DD).
    pub date: NaiveDate,
    /// Total cost in USD for the day.
    pub cost_usd: f64,
    /// Total input tokens for the day.
    pub input_tokens: u64,
    /// Total output tokens for the day.
    pub output_tokens: u64,
    /// Sum of input + output tokens for the day.
    pub total_tokens: u64,
    /// Number of recorded requests for the day.
    pub request_count: usize,
    /// Per-model aggregates for the day.
    pub by_model: std::collections::HashMap<String, ModelStats>,
}

impl DailyCostEntry {
    /// Construct an empty entry for the given date — used to fill gaps when
    /// no usage was recorded on a calendar day so charts still render the bar.
    pub fn empty(date: NaiveDate) -> Self {
        Self {
            date,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            request_count: 0,
            by_model: std::collections::HashMap::new(),
        }
    }
}

/// Budget status derived from the configured warn/alert thresholds and the
/// current month-to-date spend. Drives bar colour-coding on the dashboard.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus {
    /// Spend is below the warn threshold.
    Normal,
    /// Spend is above warn but below alert.
    Warning,
    /// Spend has reached or crossed the alert threshold.
    Exceeded,
}

/// Aggregate dashboard payload returned by `cost_get_dashboard`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostDashboard {
    /// 7-day daily entries, oldest first, gaps zero-filled.
    pub days: Vec<DailyCostEntry>,
    /// Sum of `cost_usd` across `days`.
    pub period_total_usd: f64,
    /// Projected monthly spend: daily average × 30.
    pub monthly_pace_usd: f64,
    /// Configured monthly budget limit (USD).
    pub budget_limit_monthly_usd: f64,
    /// Month-to-date spend (USD).
    pub month_to_date_usd: f64,
    /// Fraction of the monthly budget consumed (`month_to_date / limit`).
    /// Capped at 1.0 for display purposes; UIs that need overrun should
    /// recompute from `month_to_date_usd` and `budget_limit_monthly_usd`.
    pub budget_utilization: f64,
    /// Derived status based on warn/alert thresholds.
    pub budget_status: BudgetStatus,
    /// Display currency label, e.g. "USD". All amounts are stored in USD;
    /// this is purely a presentation hint.
    pub currency: String,
    /// Per-model breakdown across the 7-day window, sorted by cost desc.
    pub by_model: Vec<ModelStats>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_calculation() {
        let usage = TokenUsage::new("test/model", 1000, 500, 3.0, 15.0);

        // Expected: (1000/1M)*3 + (500/1M)*15 = 0.003 + 0.0075 = 0.0105
        assert!((usage.cost_usd - 0.0105).abs() < 0.0001);
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.total_tokens, 1500);
    }

    #[test]
    fn token_usage_zero_tokens() {
        let usage = TokenUsage::new("test/model", 0, 0, 3.0, 15.0);
        assert!(usage.cost_usd.abs() < f64::EPSILON);
        assert_eq!(usage.total_tokens, 0);
    }

    #[test]
    fn token_usage_negative_or_non_finite_prices_are_clamped() {
        let usage = TokenUsage::new("test/model", 1000, 1000, -3.0, f64::NAN);
        assert!(usage.cost_usd.abs() < f64::EPSILON);
        assert_eq!(usage.total_tokens, 2000);
    }

    #[test]
    fn cost_record_creation() {
        let usage = TokenUsage::new("test/model", 100, 50, 1.0, 2.0);
        let record = CostRecord::new("session-123", usage);

        assert_eq!(record.session_id, "session-123");
        assert!(!record.id.is_empty());
        assert_eq!(record.usage.model, "test/model");
    }
}
