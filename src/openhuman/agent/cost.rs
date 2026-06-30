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

use crate::openhuman::inference::provider::UsageInfo;

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
    // Reasoning tier — managed "Pro" model rates (estimate; the backend's
    // echoed `charged_amount_usd` is authoritative when present). Shared with
    // the coding/agentic tiers below. Update when backend pricing changes.
    ModelPricing {
        model: "reasoning-v1",
        input_per_mtok_usd: 0.435,
        cached_input_per_mtok_usd: 0.003625,
        output_per_mtok_usd: 0.87,
    },
    // Chat tier — managed "Flash" model rates (estimate). Cheaper, lower-latency
    // model used for direct conversational turns.
    ModelPricing {
        model: "chat-v1",
        input_per_mtok_usd: 0.14,
        cached_input_per_mtok_usd: 0.0028,
        output_per_mtok_usd: 0.28,
    },
    // Legacy chat tier slug retained for older transcripts/configs — "Flash"
    // rates, same as `chat-v1`.
    ModelPricing {
        model: "reasoning-quick-v1",
        input_per_mtok_usd: 0.14,
        cached_input_per_mtok_usd: 0.0028,
        output_per_mtok_usd: 0.28,
    },
    // Agentic tier — managed "Pro" model rates (same as reasoning).
    ModelPricing {
        model: "agentic-v1",
        input_per_mtok_usd: 0.435,
        cached_input_per_mtok_usd: 0.003625,
        output_per_mtok_usd: 0.87,
    },
    // Coding tier — managed "Pro" model rates (same as reasoning).
    ModelPricing {
        model: "coding-v1",
        input_per_mtok_usd: 0.435,
        cached_input_per_mtok_usd: 0.003625,
        output_per_mtok_usd: 0.87,
    },
    // Burst tier — high-throughput, low-cost model; flat rate both directions,
    // no prompt cache (so cached rate mirrors the input rate). Used by the
    // SuperContext scout.
    ModelPricing {
        model: "burst-v1",
        input_per_mtok_usd: 0.208,
        cached_input_per_mtok_usd: 0.208,
        output_per_mtok_usd: 0.208,
    },
    // Vision tier — multimodal; estimate only. The backend's echoed
    // `charged_amount_usd` is authoritative when present.
    ModelPricing {
        model: "vision-v1",
        input_per_mtok_usd: 3.00,
        cached_input_per_mtok_usd: 0.30,
        output_per_mtok_usd: 15.00,
    },
];

/// Look up pricing for a model name, falling back to [`FALLBACK_PRICING`].
///
/// Resolution order:
/// 1. Exact match on a canonical OpenHuman tier name (`agentic-v1`, …).
/// 2. The concrete-vendor-model pricing catalog
///    ([`crate::openhuman::cost::catalog`]) — accurate per-model rates for
///    `claude-*`, `gpt-*`, `gemini-*`, `deepseek-*`, `kimi-*`, `qwen-*`,
///    `mistral-*`, including OpenRouter-style `vendor/model` ids.
/// 3. Coarse case-insensitive vendor-name heuristics (so an unrecognised
///    `"…opus…"` string still maps to the reasoning tier).
/// 4. [`FALLBACK_PRICING`].
pub fn lookup_pricing(model: &str) -> ModelPricing {
    if let Some(row) = PRICING_TABLE.iter().find(|row| row.model == model) {
        return *row;
    }
    if let Some(price) = crate::openhuman::cost::catalog::lookup(model) {
        return ModelPricing {
            model: price.model_id,
            input_per_mtok_usd: price.input_per_mtok_usd,
            cached_input_per_mtok_usd: price.cached_input_per_mtok_usd,
            output_per_mtok_usd: price.output_per_mtok_usd,
        };
    }
    let lower = model.to_ascii_lowercase();
    let by_tier = |tier: &str| {
        PRICING_TABLE
            .iter()
            .find(|row| row.model == tier)
            .copied()
            .unwrap_or(FALLBACK_PRICING)
    };
    if lower.contains("opus") {
        return by_tier("reasoning-v1");
    }
    if lower.contains("coding") {
        return by_tier("coding-v1");
    }
    if lower.contains("sonnet") || lower.contains("agentic") {
        return by_tier("agentic-v1");
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
        // Reasoning/agentic share the managed "Pro" rates.
        assert_eq!(lookup_pricing("reasoning-v1").input_per_mtok_usd, 0.435);
        assert_eq!(lookup_pricing("agentic-v1").output_per_mtok_usd, 0.87);
    }

    #[test]
    fn lookup_pricing_has_a_vision_row() {
        // The vision tier must price exactly (not via the fallback) so budget
        // gating bites correctly. See PR adding the `vision-v1` tier.
        let p = lookup_pricing("vision-v1");
        assert_eq!(p.model, "vision-v1");
        assert_eq!(p.output_per_mtok_usd, 15.0);
    }

    #[test]
    fn lookup_pricing_has_a_burst_row() {
        // The burst tier (SuperContext scout) must price from its own row —
        // NOT via the $3/$15 fallback, which would inflate first-turn scout cost
        // and could trip budget gates.
        let p = lookup_pricing("burst-v1");
        assert_eq!(p.model, "burst-v1");
        assert_eq!(p.input_per_mtok_usd, 0.208);
        assert_eq!(p.output_per_mtok_usd, 0.208);
    }

    #[test]
    fn lookup_pricing_falls_back_for_unknown_model() {
        let p = lookup_pricing("totally-unknown-model");
        assert_eq!(p.model, "<fallback>");
    }

    #[test]
    fn lookup_pricing_handles_concrete_vendor_names() {
        // `claude-opus-4.7` (dotted, not a catalog id) resolves via the `opus`
        // vendor heuristic to the reasoning tier ("Pro" rates).
        assert_eq!(lookup_pricing("claude-opus-4.7").input_per_mtok_usd, 0.435);
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
        // 1M standard input + 1M cached input + 1M output on agentic-v1 ("Pro").
        let u = usage(2_000_000, 1_000_000, 1_000_000, 0.0);
        let est = estimate_call_cost_usd("agentic-v1", &u);
        // 1M*0.435 + 1M*0.003625 + 1M*0.87 = 1.308625
        assert!((est - 1.308625).abs() < 1e-6, "got {est}");
    }

    #[test]
    fn call_cost_prefers_charged_when_present() {
        let u = usage(100_000, 200_000, 0, 0.42);
        assert_eq!(call_cost_usd("reasoning-v1", &u), 0.42);
    }

    #[test]
    fn call_cost_falls_back_to_estimate_when_charged_zero() {
        let u = usage(1_000_000, 0, 0, 0.0);
        // 1M input * 0.435 = 0.435
        assert!((call_cost_usd("agentic-v1", &u) - 0.435).abs() < 1e-6);
    }

    #[test]
    fn turn_cost_accumulates_charged_and_estimated_separately() {
        let mut tc = TurnCost::new();
        tc.add_call("reasoning-v1", &usage(0, 0, 0, 0.10));
        tc.add_call("agentic-v1", &usage(1_000_000, 0, 0, 0.0)); // est: 0.435
        assert_eq!(tc.call_count, 2);
        assert!((tc.charged_usd - 0.10).abs() < 1e-6);
        assert!((tc.estimated_usd - 0.435).abs() < 1e-6);
        assert!((tc.total_usd() - 0.535).abs() < 1e-6);
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
