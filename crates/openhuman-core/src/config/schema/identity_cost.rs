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
    /// Retained for **recording**, not enforcement: on its own it refuses
    /// nothing (only opt-in [`Self::budgets`] can). `CostTracker::record_usage` is a no-op when this is
    /// `false`; `record_usage_unconditional` (the dashboard/telemetry path)
    /// ignores it.
    ///
    /// Dashboard telemetry uses `record_usage_unconditional`, so this flag
    /// does not disable telemetry capture. The dashboard
    /// JSONL store at `{workspace}/state/costs.jsonl` is populated by
    /// [`crate::platform::cost::record_provider_usage`] regardless of
    /// this flag, so users can review historical usage. Set
    /// `dashboard.enabled = false` to hide the
    /// Settings panel; delete the JSONL file to clear collected
    /// history. The file is local and never leaves the workspace.
    #[serde(default = "default_cost_enabled")]
    pub enabled: bool,

    /// Legacy monthly display target in USD (default: 100.00).
    ///
    /// **This is a display target, not a cap.** Nothing in the core refuses a
    /// request when it is exceeded — the enforcement path was removed with the
    /// spend cap. It remains in the dashboard RPC payload for compatibility,
    /// but the UI no longer presents it as a limit.
    ///
    /// Counts **managed (OpenHuman-credit) spend only** — see
    /// [`crate::platform::cost::route`]. Bring-your-own-key and local
    /// inference is billed by the user's own provider, so it is recorded for
    /// the dashboard but never counted here (#5016); driving the gauge off the
    /// all-route total filled a pure-BYOK user's bar against a limit that
    /// could never fire.
    ///
    /// A retired `daily_limit_usd` key may still be present in existing config
    /// files. It is accepted and ignored — this struct does not
    /// `deny_unknown_fields`, so upgrading never fails to parse.
    #[serde(default = "default_monthly_limit")]
    pub monthly_limit_usd: f64,

    /// Per-model pricing (USD per 1M tokens)
    #[serde(default)]
    pub prices: HashMap<String, ModelPricing>,

    /// Dashboard chart panel configuration. Drives the 7-day cost / token
    /// visualisation in Settings → Cost dashboard.
    #[serde(default)]
    pub dashboard: CostDashboardConfig,

    /// Spend and token budgets, checked before every model call
    /// (`platform::cost::budget`). Empty by default: nothing is limited until
    /// a budget is configured.
    #[serde(default)]
    pub budgets: Vec<BudgetPolicy>,
}

/// One budget: a limit on spend and/or tokens over a period, for every call
/// or for each thread, agent, model or user agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BudgetPolicy {
    /// A label for logs and refusals.
    #[serde(default)]
    pub name: Option<String>,
    /// What the limit applies to.
    #[serde(default)]
    pub scope: BudgetScope,
    /// Apply only to this value of `scope` (one agent id, one model, …).
    /// Unset applies the limit to each value separately.
    #[serde(default, rename = "match")]
    pub matches: Option<String>,
    /// The window spend is summed over.
    #[serde(default)]
    pub period: BudgetPeriod,
    /// Spend limit in USD.
    #[serde(default)]
    pub max_usd: Option<f64>,
    /// Token limit (input + output).
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// Fraction of a limit at which a warning is logged (default 0.8).
    #[serde(default = "default_budget_warn_fraction")]
    pub warn_fraction: f64,
    /// What happens once a limit is reached.
    #[serde(default)]
    pub action: BudgetAction,
}

/// What a [`BudgetPolicy`] applies to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScope {
    /// Every call together.
    #[default]
    Global,
    Thread,
    /// The agent definition making the call.
    Agent,
    Model,
    /// The embedded or SaaS user agent.
    SessionAgent,
}

/// The window a [`BudgetPolicy`] sums spend over (UTC).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    /// Since midnight UTC.
    Day,
    /// Since the first of the month, UTC.
    #[default]
    Month,
}

/// What reaching a [`BudgetPolicy`] limit does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAction {
    /// Log a warning and let the call through.
    #[default]
    Warn,
    /// Refuse the call with `BUDGET_EXCEEDED`.
    Refuse,
}

fn default_budget_warn_fraction() -> f64 {
    0.8
}

/// Configuration for the 7-day cost & token usage dashboard panel.
///
/// Legacy thresholds are retained in the dashboard RPC payload for
/// compatibility; the UI does not display budget warnings.
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

fn default_monthly_limit() -> f64 {
    100.0
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
        let prices = crate::inference::registry::provider_default_pricing(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        )
        .map(|(model, input, output)| HashMap::from([(model, ModelPricing { input, output })]))
        .unwrap_or_default();
        Self {
            enabled: default_cost_enabled(),
            monthly_limit_usd: default_monthly_limit(),
            prices,
            dashboard: CostDashboardConfig::default(),
            budgets: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "identity_cost_tests.rs"]
mod tests;
