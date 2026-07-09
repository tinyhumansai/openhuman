//! 20:1 compression mechanics for the `compress` node (stage 5).
//!
//! The node condenses the cycle's execution trace into a single compressed
//! entry. The **budget** and **enforcement** are pure, deterministic functions
//! (unit-tested here); the LLM summarization call + store write live in the
//! production runtime ([`super::super::ops`]).
//!
//! Global invariant (spec §5): the output budget is `input_tokens / 20`,
//! enforced — not advisory.

use tinyagents::harness::summarization::estimate_tokens;

/// The strict compression ratio (spec §3): 20 input tokens per output token.
pub const COMPRESSION_RATIO: u64 = 20;

/// Minimum output budget, applied only when the source is large enough that the
/// floor is still compressive (floor < input).
pub const COMPRESSION_FLOOR_TOKENS: u64 = 200;

/// Estimate the token count of `text` using the same heuristic `summarize.rs` uses.
pub fn count_tokens(text: &str) -> u64 {
    estimate_tokens(text)
}

/// The enforced output budget for a trace of `input_tokens`:
/// `min(input_tokens / 20, input_tokens)`, with the 200-token floor applied only
/// when the source is large enough that the floor stays compressive
/// (`floor < input_tokens`). A tiny source keeps its sub-floor ratio budget so a
/// short trace is not *expanded* up to the floor.
pub fn compression_budget(input_tokens: u64) -> u64 {
    if input_tokens == 0 {
        return 0;
    }
    let ratio_budget = (input_tokens / COMPRESSION_RATIO).max(1);
    let budget =
        if ratio_budget < COMPRESSION_FLOOR_TOKENS && COMPRESSION_FLOOR_TOKENS < input_tokens {
            COMPRESSION_FLOOR_TOKENS
        } else {
            ratio_budget
        };
    budget.min(input_tokens)
}

/// Enforce the output budget on a produced `summary`. If it exceeds 1.5× the
/// budget, hard-truncate to roughly the budget token count. Returns the enforced
/// text and whether it was truncated (the caller retries once before accepting a
/// truncation — see the runtime).
pub fn enforce_budget(summary: &str, budget_tokens: u64) -> (String, bool) {
    let tokens = estimate_tokens(summary);
    if tokens <= budget_tokens.saturating_mul(3) / 2 {
        return (summary.to_string(), false);
    }
    // `estimate_tokens` is ~chars/4; truncate by the proportional char count.
    let keep_ratio = budget_tokens as f64 / tokens.max(1) as f64;
    let total_chars = summary.chars().count();
    let keep_chars = ((total_chars as f64) * keep_ratio).floor() as usize;
    let truncated: String = summary.chars().take(keep_chars.max(1)).collect();
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_strict_20_to_1_for_large_traces() {
        // 4000 input → 200 budget (exactly 20:1; floor not needed).
        assert_eq!(compression_budget(4000), 200);
        // 20000 input → 1000 budget.
        assert_eq!(compression_budget(20_000), 1000);
    }

    #[test]
    fn floor_applies_only_when_still_compressive() {
        // 1000 input → 20:1 = 50, below the 200 floor, and 200 < 1000 → floor.
        assert_eq!(compression_budget(1000), 200);
        // 100 input → 20:1 = 5; the 200 floor would EXPAND it, so keep the ratio.
        assert_eq!(compression_budget(100), 5);
        // 0 input → 0 (nothing to compress).
        assert_eq!(compression_budget(0), 0);
    }

    #[test]
    fn enforce_hard_truncates_when_over_one_and_a_half_budget() {
        // A summary well over 1.5× budget must be truncated to ≈ budget.
        let long = "word ".repeat(4000); // ~4000 words, thousands of tokens
        let budget = 100;
        let (out, truncated) = enforce_budget(&long, budget);
        assert!(truncated, "over-budget summary must be truncated");
        assert!(
            count_tokens(&out) <= budget * 3 / 2,
            "enforced output {} tokens must be ≤ budget×1.5 = {}",
            count_tokens(&out),
            budget * 3 / 2
        );
    }

    #[test]
    fn enforce_leaves_within_budget_summaries_untouched() {
        let short = "a concise summary";
        let (out, truncated) = enforce_budget(short, 100);
        assert!(!truncated);
        assert_eq!(out, short);
    }
}
