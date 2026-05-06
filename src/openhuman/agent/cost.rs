//! Per-turn cost accounting for an agent's tool-call loop.
//!
//! Each provider response carries an optional [`UsageInfo`] block with
//! `input_tokens`, `output_tokens`, `cached_input_tokens`, and an
//! authoritative `charged_amount_usd` populated by the OpenHuman
//! backend. [`TurnCost`] sums those across every provider call inside a
//! single turn so the harness can:
//!
//! - emit per-iteration cost telemetry via
//!   [`crate::openhuman::agent::progress::AgentProgress::TurnCostUpdated`];
//! - feed an upcoming budget stop-hook (mid-turn USD cap);
//! - log accurate end-of-turn cost lines.
//!
//! When `charged_amount_usd` is zero (older backend builds, providers
//! that don't surface billing), we fall back to a simple token-rate
//! estimate via [`estimate_call_cost_usd`] keyed on the model tier
//! name. The estimate is a floor — directly-billed cost from the
//! backend always wins when available.
//!
//! The pricing table is intentionally tiny and only keyed on the
//! abstract tier names the core uses (`agentic-v1`, `reasoning-v1`,
//! `coding-v1`). The backend resolves them to concrete vendor models;
//! cents-per-Mtok at the tier level is good enough for client-side
//! telemetry and budget gating. PRs adding new tiers should add a row.

use crate::openhuman::providers::UsageInfo;

/// Per-million-token rates for a single model tier.
///
/// All prices are USD per million tokens. `cached_input_per_mtok_usd`
/// applies to the `cached_input_tokens` portion of the usage block (KV
/// prefix cache hits on supporting backends); the remaining
/// `input_tokens - cached_input_tokens` are charged at
/// `input_per_mtok_usd`.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    /// Tier identifier, e.g. `"agentic-v1"`.
    pub model: &'static str,
    /// Standard prompt rate, USD per million input tokens.
    pub input_per_mtok_usd: f64,
    /// Cached-prefix prompt rate, USD per million cached input tokens.
    pub cached_input_per_mtok_usd: f64,
    /// Completion rate, USD per million output tokens.
    pub output_per_mtok_usd: f64,
}

/// Conservative fallback when nothing in the table matches. Picked so
/// budget caps still bite on unknown models rather than reading as $0.
const FALLBACK_PRICING: ModelPricing = ModelPricing {
    model: "<fallback>",
    input_per_mtok_usd: 3.00,
    cached_input_per_mtok_usd: 0.30,
    output_per_mtok_usd: 15.00,
};

/// Static price table keyed by tier name.
///
/// These are the OpenHuman tier handles, not concrete vendor model
/// strings — the backend chooses which underlying Claude / GPT / etc.
/// model serves each tier. Numbers track the public Anthropic price
/// list at the time of writing for the tiers' default mappings; treat
/// them as best-effort estimates for cases where the backend doesn't
/// echo `charged_amount_usd`.
pub const PRICING_TABLE: &[ModelPricing] = &[
    // Reasoning tier — currently maps to Claude Opus 4.x family.
    ModelPricing {
        model: "reasoning-v1",
        input_per_mtok_usd: 15.00,
        cached_input_per_mtok_usd: 1.50,
        output_per_mtok_usd: 75.00,
    },
    // Agentic tier — maps to Sonnet-class models.
    ModelPricing {
        model: "agentic-v1",
        input_per_mtok_usd: 3.00,
        cached_input_per_mtok_usd: 0.30,
        output_per_mtok_usd: 15.00,
    },
    // Coding tier — Sonnet-class.
    ModelPricing {
        model: "coding-v1",
        input_per_mtok_usd: 3.00,
        cached_input_per_mtok_usd: 0.30,
        output_per_mtok_usd: 15.00,
    },
];

/// Look up pricing for a model name, falling back to [`FALLBACK_PRICING`].
///
/// Matching is exact on the canonical tier name and case-insensitive on
/// concrete vendor names (so `"claude-opus"` still hits the
/// reasoning-tier row when callers pass an underlying model string).
pub fn lookup_pricing(model: &str) -> ModelPricing {
    if let Some(row) = PRICING_TABLE.iter().find(|row| row.model == model) {
        return *row;
    }
    let lower = model.to_ascii_lowercase();
    if lower.contains("opus") {
        return PRICING_TABLE[0];
    }
    if lower.contains("coding") {
        return PRICING_TABLE[2];
    }
    if lower.contains("sonnet") || lower.contains("agentic") {
        return PRICING_TABLE[1];
    }
    FALLBACK_PRICING
}

/// Estimate the USD cost of a single provider call from its token
/// usage. Used as a fallback when `charged_amount_usd` is missing.
pub fn estimate_call_cost_usd(model: &str, usage: &UsageInfo) -> f64 {
    let pricing = lookup_pricing(model);
    let cached = usage.cached_input_tokens;
    let standard_input = usage.input_tokens.saturating_sub(cached);
    let m = 1_000_000.0_f64;
    (standard_input as f64) / m * pricing.input_per_mtok_usd
        + (cached as f64) / m * pricing.cached_input_per_mtok_usd
        + (usage.output_tokens as f64) / m * pricing.output_per_mtok_usd
}

/// Pick the most authoritative USD figure for a single provider call.
///
/// Backend-reported `charged_amount_usd` wins whenever it's > 0;
/// otherwise we fall back to [`estimate_call_cost_usd`].
pub fn call_cost_usd(model: &str, usage: &UsageInfo) -> f64 {
    if usage.charged_amount_usd > 0.0 {
        usage.charged_amount_usd
    } else {
        estimate_call_cost_usd(model, usage)
    }
}

/// Running cost / token tally across every provider call inside a
/// single turn of the tool-call loop.
///
/// `charged_usd` is the sum of authoritative `charged_amount_usd`
/// values; `estimated_usd` adds the fallback estimate for any call that
/// lacked one. `total_usd()` returns whichever has more signal.
#[derive(Debug, Clone, Default)]
pub struct TurnCost {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub charged_usd: f64,
    pub estimated_usd: f64,
    pub call_count: u32,
}

impl TurnCost {
    /// New empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold a single provider call's usage into the running totals.
    pub fn add_call(&mut self, model: &str, usage: &UsageInfo) {
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(usage.cached_input_tokens);
        if usage.charged_amount_usd > 0.0 {
            self.charged_usd += usage.charged_amount_usd;
        } else {
            self.estimated_usd += estimate_call_cost_usd(model, usage);
        }
        self.call_count = self.call_count.saturating_add(1);
    }

    /// Best-available USD figure: authoritative charged amount plus
    /// estimated cost for any calls that didn't carry one.
    pub fn total_usd(&self) -> f64 {
        self.charged_usd + self.estimated_usd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64, cached: u64, charged: f64) -> UsageInfo {
        UsageInfo {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: cached,
            charged_amount_usd: charged,
            ..Default::default()
        }
    }

    #[test]
    fn lookup_pricing_matches_canonical_tiers() {
        assert_eq!(lookup_pricing("reasoning-v1").input_per_mtok_usd, 15.0);
        assert_eq!(lookup_pricing("agentic-v1").output_per_mtok_usd, 15.0);
    }

    #[test]
    fn lookup_pricing_falls_back_for_unknown_model() {
        let p = lookup_pricing("totally-unknown-model");
        assert_eq!(p.model, "<fallback>");
    }

    #[test]
    fn lookup_pricing_handles_concrete_vendor_names() {
        assert_eq!(lookup_pricing("claude-opus-4.7").input_per_mtok_usd, 15.0);
        assert_eq!(
            lookup_pricing("claude-sonnet-4-6").output_per_mtok_usd,
            15.0
        );
    }

    #[test]
    fn lookup_pricing_routes_coding_to_coding_row_not_agentic() {
        // Pinned per CodeRabbit feedback: when the coding-tier row
        // diverges from agentic, "coding" model strings must hit
        // PRICING_TABLE[2], not [1].
        assert_eq!(lookup_pricing("coding-v1").model, "coding-v1");
        assert_eq!(lookup_pricing("agentic-v1").model, "agentic-v1");
    }

    #[test]
    fn estimate_call_cost_subtracts_cached_input() {
        // 1M standard input + 1M cached input + 1M output on agentic-v1.
        let u = usage(2_000_000, 1_000_000, 1_000_000, 0.0);
        let est = estimate_call_cost_usd("agentic-v1", &u);
        // 1M * 3 + 1M * 0.3 + 1M * 15 = 18.3
        assert!((est - 18.3).abs() < 1e-6, "got {est}");
    }

    #[test]
    fn call_cost_prefers_charged_when_present() {
        let u = usage(100_000, 200_000, 0, 0.42);
        assert_eq!(call_cost_usd("reasoning-v1", &u), 0.42);
    }

    #[test]
    fn call_cost_falls_back_to_estimate_when_charged_zero() {
        let u = usage(1_000_000, 0, 0, 0.0);
        // 1M input * 3 = 3
        assert!((call_cost_usd("agentic-v1", &u) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn turn_cost_accumulates_charged_and_estimated_separately() {
        let mut tc = TurnCost::new();
        tc.add_call("reasoning-v1", &usage(0, 0, 0, 0.10));
        tc.add_call("agentic-v1", &usage(1_000_000, 0, 0, 0.0)); // est: 3.00
        assert_eq!(tc.call_count, 2);
        assert!((tc.charged_usd - 0.10).abs() < 1e-6);
        assert!((tc.estimated_usd - 3.0).abs() < 1e-6);
        assert!((tc.total_usd() - 3.10).abs() < 1e-6);
    }

    #[test]
    fn turn_cost_aggregates_token_counts() {
        let mut tc = TurnCost::new();
        tc.add_call("agentic-v1", &usage(100, 50, 20, 0.0));
        tc.add_call("agentic-v1", &usage(200, 75, 0, 0.0));
        assert_eq!(tc.input_tokens, 300);
        assert_eq!(tc.output_tokens, 125);
        assert_eq!(tc.cached_input_tokens, 20);
    }
}
