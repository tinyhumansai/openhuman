//! Cost tracking configuration.
//!
//! Identity is loaded from OpenClaw markdown files in the workspace
//! (`IDENTITY.md`, `SOUL.md`, etc.) and needs no config surface.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CostConfig {
    /// Enable budget enforcement (default: true).
    ///
    /// When `true`, [`crate::openhuman::cost::CostTracker::check_budget`]
    /// honours `daily_limit_usd` / `monthly_limit_usd` and refuses
    /// over-budget requests via `BudgetCheck::Exceeded`.
    ///
    /// **Important:** as of the cost-dashboard PR this flag controls
    /// **enforcement only**, not telemetry capture. The dashboard
    /// JSONL store at `{workspace}/state/costs.jsonl` is populated by
    /// [`crate::openhuman::cost::record_provider_usage`] regardless of
    /// this flag, so users can review historical spend before opting
    /// into hard caps. Set `dashboard.enabled = false` to hide the
    /// Settings panel; delete the JSONL file to clear collected
    /// history. The file is local and never leaves the workspace.
    #[serde(default = "default_cost_enabled")]
    pub enabled: bool,

    /// Daily spending limit in USD (default: 10.00)
    #[serde(default = "default_daily_limit")]
    pub daily_limit_usd: f64,

    /// Monthly spending limit in USD (default: 100.00)
    #[serde(default = "default_monthly_limit")]
    pub monthly_limit_usd: f64,

    /// Warn when spending reaches this percentage of limit (default: 80)
    #[serde(default = "default_warn_percent")]
    pub warn_at_percent: u8,

    /// Per-model pricing (USD per 1M tokens)
    #[serde(default)]
    pub prices: HashMap<String, ModelPricing>,

    /// Dashboard chart panel configuration. Drives the 7-day cost / token
    /// visualisation in Settings → Cost dashboard.
    #[serde(default)]
    pub dashboard: CostDashboardConfig,
}

/// Configuration for the 7-day cost & token usage dashboard panel.
///
/// The monthly budget itself is read from [`CostConfig::monthly_limit_usd`]
/// — `warn_threshold` and `alert_threshold` are fractions of that budget
/// that drive bar colour-coding and status badges on the chart.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CostDashboardConfig {
    /// Whether the dashboard panel is enabled in the UI. The panel still
    /// renders a disabled hint when this is false.
    #[serde(default = "default_dashboard_enabled")]
    pub enabled: bool,

    /// Display currency label. Amounts are always stored in USD; this is
    /// purely a presentation hint.
    #[serde(default = "default_currency")]
    pub currency: String,

    /// Warn threshold as a fraction of the monthly budget (default: 0.8).
    /// Bars and status flip to amber once month-to-date utilisation reaches
    /// this value.
    #[serde(default = "default_warn_threshold")]
    pub warn_threshold: f64,

    /// Alert threshold as a fraction of the monthly budget (default: 0.95).
    /// Bars and status flip to red once month-to-date utilisation reaches
    /// this value.
    #[serde(default = "default_alert_threshold")]
    pub alert_threshold: f64,
}

impl Default for CostDashboardConfig {
    fn default() -> Self {
        Self {
            enabled: default_dashboard_enabled(),
            currency: default_currency(),
            warn_threshold: default_warn_threshold(),
            alert_threshold: default_alert_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelPricing {
    /// Input price per 1M tokens
    #[serde(default)]
    pub input: f64,

    /// Output price per 1M tokens
    #[serde(default)]
    pub output: f64,
}

fn default_cost_enabled() -> bool {
    true
}

fn default_daily_limit() -> f64 {
    10.0
}

fn default_monthly_limit() -> f64 {
    100.0
}

fn default_warn_percent() -> u8 {
    80
}

fn default_dashboard_enabled() -> bool {
    true
}

fn default_currency() -> String {
    "USD".to_string()
}

fn default_warn_threshold() -> f64 {
    0.8
}

fn default_alert_threshold() -> f64 {
    0.95
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            enabled: default_cost_enabled(),
            daily_limit_usd: default_daily_limit(),
            monthly_limit_usd: default_monthly_limit(),
            warn_at_percent: default_warn_percent(),
            prices: get_default_pricing(),
            dashboard: CostDashboardConfig::default(),
        }
    }
}

/// Default pricing for popular models (USD per 1M tokens)
fn get_default_pricing() -> HashMap<String, ModelPricing> {
    use super::types::{
        MODEL_AGENTIC_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1, MODEL_REASONING_V1,
    };

    let mut prices = HashMap::new();

    prices.insert(
        MODEL_REASONING_V1.into(),
        ModelPricing {
            input: 0.84,
            output: 2.52,
        },
    );
    // Kimi K2.6 Turbo on Fireworks — see backend PR #760.
    prices.insert(
        MODEL_REASONING_QUICK_V1.into(),
        ModelPricing {
            input: 0.60,
            output: 2.50,
        },
    );
    prices.insert(
        MODEL_AGENTIC_V1.into(),
        ModelPricing {
            input: 0.45,
            output: 1.80,
        },
    );
    prices.insert(
        MODEL_CODING_V1.into(),
        ModelPricing {
            input: 0.90,
            output: 3.30,
        },
    );

    prices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_config_defaults() {
        let c = CostConfig::default();
        assert!(c.enabled);
        assert_eq!(c.daily_limit_usd, 10.0);
        assert_eq!(c.monthly_limit_usd, 100.0);
        assert_eq!(c.warn_at_percent, 80);
        assert!(!c.prices.is_empty());
        assert!(c.dashboard.enabled);
        assert_eq!(c.dashboard.currency, "USD");
        assert!((c.dashboard.warn_threshold - 0.8).abs() < f64::EPSILON);
        assert!((c.dashboard.alert_threshold - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_dashboard_config_serde_roundtrip() {
        let toml = r#"
            enabled = true
            [dashboard]
            enabled = false
            currency = "EUR"
            warn_threshold = 0.5
            alert_threshold = 0.9
        "#;
        let c: CostConfig = toml::from_str(toml).unwrap();
        assert!(!c.dashboard.enabled);
        assert_eq!(c.dashboard.currency, "EUR");
        assert!((c.dashboard.warn_threshold - 0.5).abs() < f64::EPSILON);
        assert!((c.dashboard.alert_threshold - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_config_default_pricing_has_known_models() {
        let c = CostConfig::default();
        assert!(c.prices.len() >= 3);
    }

    #[test]
    fn cost_config_serde_roundtrip() {
        let c = CostConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let back: CostConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.daily_limit_usd, 10.0);
        assert_eq!(back.monthly_limit_usd, 100.0);
    }

    #[test]
    fn cost_config_toml_with_custom_values() {
        let toml = r#"
            enabled = true
            daily_limit_usd = 50.0
            monthly_limit_usd = 500.0
            warn_at_percent = 90
        "#;
        let c: CostConfig = toml::from_str(toml).unwrap();
        assert!(c.enabled);
        assert_eq!(c.daily_limit_usd, 50.0);
        assert_eq!(c.monthly_limit_usd, 500.0);
        assert_eq!(c.warn_at_percent, 90);
    }

    #[test]
    fn model_pricing_defaults_to_zero() {
        let p: ModelPricing = serde_json::from_str("{}").unwrap();
        assert_eq!(p.input, 0.0);
        assert_eq!(p.output, 0.0);
    }
}
