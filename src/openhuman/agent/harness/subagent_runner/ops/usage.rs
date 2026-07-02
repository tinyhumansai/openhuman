//! Cumulative sub-agent usage stats.
//!
//! The sub-agent inner tool-call loop that produced these was retired in favour
//! of the tinyagents harness (issue #4249); only the usage aggregate it returned
//! survives, reused by the tinyagents sub-agent route ([`super::graph`]).

/// Cumulative usage stats gathered across a sub-agent run.
#[derive(Debug, Clone, Default)]
pub(super) struct AggregatedUsage {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) charged_amount_usd: f64,
}
