//! Tunable constants for STM recall — the STM/LTM knobs.
//!
//! Split out of `mod.rs` per the repo's "light `mod.rs`" rule (CLAUDE.md):
//! `mod.rs` stays export-focused; operational/tunable values live here and
//! are re-exported (`pub use constants::*;`) so existing `super::CONST`
//! references in `recall.rs` / `tool.rs` keep working unchanged.

/// STM recency window in days. Segments or episodic entries older than this
/// are excluded — they belong in LTM (the memory tree).
pub const RECENCY_WINDOW_DAYS: f64 = 14.0;

/// Hard cap on segments loaded for vector search (Arm 2).
/// Keeps the brute-force cosine pass bounded at single-user scale.
pub const RECENCY_WINDOW_MAX_SEGMENTS: usize = 100;

/// Cosine similarity gate for Arm 2 (segment recaps).
/// Below this threshold a recap is excluded regardless of recency.
/// Range: [0.0, 1.0]; 0.65 is "medium gate" — confident topical overlap.
pub const COSINE_GATE: f32 = 0.65;

/// Maximum segment recaps to include in the output block.
pub const MAX_SEGMENT_RECAPS: usize = 5;

/// Maximum raw episodic turns to include in the output block.
pub const MAX_EPISODIC_TURNS: usize = 5;

/// Approximate token budget for the entire STM block (chars / 4 ≈ tokens).
/// ~1500 tokens × 4 chars/token = 6000 chars.
pub const TOKEN_BUDGET: usize = 6_000;

/// How many FTS5 candidates to fetch before applying the high-precision gate.
/// The gate is: only strong keyword matches survive — FTS5 rank threshold is
/// applied at the DB level via LIMIT; we over-fetch slightly and let dedup
/// finish trimming.
pub const FTS5_LIMIT: usize = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stm_recall_constants_match_expected_defaults() {
        assert_eq!(RECENCY_WINDOW_DAYS, 14.0);
        assert_eq!(RECENCY_WINDOW_MAX_SEGMENTS, 100);
        assert_eq!(COSINE_GATE, 0.65);
        assert_eq!(MAX_SEGMENT_RECAPS, 5);
        assert_eq!(MAX_EPISODIC_TURNS, 5);
        assert_eq!(TOKEN_BUDGET, 6_000);
        assert_eq!(FTS5_LIMIT, 20);
    }

    #[test]
    fn stm_token_budget_exceeds_single_section_limits() {
        assert!(TOKEN_BUDGET > MAX_SEGMENT_RECAPS);
        assert!(TOKEN_BUDGET > MAX_EPISODIC_TURNS);
    }
}
