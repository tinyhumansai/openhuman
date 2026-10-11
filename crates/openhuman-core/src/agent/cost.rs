//! Per-call and per-turn cost accounting for an agent's tool-call loop.
//!
//! A call's cost is one of three things, never a guess:
//!
//! - **Charged**: the amount the provider actually billed, reported back on
//!   the response (`openhuman.billing.charged_amount_usd` from the managed
//!   backend, `total_cost_usd` from a CLI provider). It may be zero for a
//!   free route; that is still a known cost.
//! - **Estimated**: no charge was reported, but the model is in the vendor
//!   price catalog ([`crate::platform::cost::catalog`]), so its published
//!   list rates price the token counts. Reported as an estimate.
//! - **Unknown**: no charge and no catalogued price. Nothing is reported for
//!   it; a turn with any unknown call has an unknown total.
//!
//! There is deliberately no default rate for an unrecognised model. One used
//! to exist ($3 / $0.30 / $15 per MTok) and priced a glm-5.3-flash thread at
//! $4.25 when the provider billed about $0.30.

use crate::inference::provider::BilledUsage;

/// Whether a model names an explicitly free OpenRouter variant.
pub(crate) fn is_free_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().ends_with(":free")
}

/// Supply product catalog rates as data to the harness accounting API.
pub(crate) fn lookup_pricing(model: &str) -> Option<tinyagents_harness::cost::ModelPricing> {
    if is_free_model(model) {
        return Some(tinyagents_harness::cost::ModelPricing {
            input_per_token: Some(0.0),
            cache_read_input_per_token: Some(0.0),
            output_per_token: Some(0.0),
            ..Default::default()
        });
    }
    let price = tinyagents_registry::pricing::lookup(model.trim())?;
    Some(tinyagents_registry::pricing::flat_catalog_entry(price, "openhuman-cost-catalog").pricing)
}

/// Estimate a call from product list prices when no provider charge was reported.
pub fn estimate_call_cost_usd(model: &str, usage: &BilledUsage) -> Option<f64> {
    let pricing = lookup_pricing(model)?;
    let usage = tinyinference_llm::usage::Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_tokens: usage.cached_input_tokens().min(usage.input_tokens),
        ..Default::default()
    };
    Some(tinyagents_harness::cost::estimate_cost(&pricing, &usage).total_cost)
}

/// Where a cost figure came from. Ordered from most to least certain, so the
/// cost of several calls takes the least certain of their sources.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    /// Every call reported the amount the provider billed.
    #[default]
    Charged,
    /// At least one call was priced from the catalog's list rates.
    Estimated,
    /// At least one call had neither a charge nor a catalogued price, so the
    /// total is not known.
    Unknown,
}

/// The cost of one provider call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CallCost {
    /// Billed by the provider (may be zero).
    Charged(f64),
    /// Catalog list-price estimate.
    Estimated(f64),
    /// Neither available.
    Unknown,
}

impl CallCost {
    /// The USD figure, or `None` when unknown.
    pub fn usd(self) -> Option<f64> {
        match self {
            Self::Charged(usd) | Self::Estimated(usd) => Some(usd),
            Self::Unknown => None,
        }
    }

    /// The source this cost contributes to a total.
    pub fn source(self) -> CostSource {
        match self {
            Self::Charged(_) => CostSource::Charged,
            Self::Estimated(_) => CostSource::Estimated,
            Self::Unknown => CostSource::Unknown,
        }
    }
}

/// The cost of one call: the provider's charge when it reported one, else
/// the catalog estimate, else unknown.
pub fn call_cost(model: &str, usage: &BilledUsage) -> CallCost {
    if usage.charge_reported && !usage.cost_is_estimate {
        return CallCost::Charged(usage.charged_amount_usd);
    }
    match estimate_call_cost_usd(model, usage) {
        Some(usd) => CallCost::Estimated(usd),
        None => CallCost::Unknown,
    }
}

/// A sum of call costs that remembers how certain it is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CostTally {
    /// Sum of the known call costs. Meaningless on its own once `source` is
    /// [`CostSource::Unknown`]; read it through [`CostTally::usd`].
    pub known_usd: f64,
    /// The least certain source among the calls added.
    pub source: CostSource,
}

impl CostTally {
    /// Fold one call in.
    pub fn add(&mut self, cost: CallCost) {
        self.known_usd += cost.usd().unwrap_or(0.0);
        self.source = self.source.max(cost.source());
    }

    /// Fold another tally in (a sub-step's or a child's).
    pub fn merge(&mut self, other: CostTally) {
        self.known_usd += other.known_usd;
        self.source = self.source.max(other.source);
    }

    /// The total, or `None` when any call's cost was unknown.
    pub fn usd(&self) -> Option<f64> {
        (self.source != CostSource::Unknown).then_some(self.known_usd)
    }
}

/// Running cost / token tally across every provider call inside a
/// single turn of the tool-call loop.
#[derive(Debug, Clone, Default)]
pub struct TurnCost {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost: CostTally,
    pub call_count: u32,
}

impl TurnCost {
    /// New empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold a single provider call's usage into the running totals.
    pub fn add_call(&mut self, model: &str, usage: &BilledUsage) {
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(usage.cached_input_tokens());
        self.cost.add(call_cost(model, usage));
        self.call_count = self.call_count.saturating_add(1);
    }

    /// The turn's cost so far, or `None` when any call's cost is unknown.
    pub fn total_usd(&self) -> Option<f64> {
        self.cost.usd()
    }
}

#[cfg(test)]
#[path = "cost_tests.rs"]
mod tests;
