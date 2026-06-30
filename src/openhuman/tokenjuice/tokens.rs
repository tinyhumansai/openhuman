//! Lightweight token estimation for compaction savings accounting.
//!
//! The agent harness gets authoritative token counts from the provider, but
//! the content router runs *before* any provider call, so it has no exact
//! count for a tool result. For savings insights we use the standard ~4
//! characters-per-token heuristic (close enough for English text / code /
//! JSON to drive cost estimates and the savings dashboard). This is the same
//! order of approximation Headroom reports its savings with.

/// Average characters per token used by the estimate.
pub const CHARS_PER_TOKEN: f64 = 4.0;

/// Estimate the number of tokens in `text` (≈ chars / 4, minimum 1 for any
/// non-empty input). Uses `chars().count()` so multi-byte text isn't
/// over-counted by byte length.
pub fn estimate_tokens(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count() as f64;
    (chars / CHARS_PER_TOKEN).ceil().max(1.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn rough_quarter_of_chars() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
        // Any non-empty input is at least one token.
        assert_eq!(estimate_tokens("a"), 1);
    }

    #[test]
    fn counts_chars_not_bytes() {
        // 4 multi-byte chars → 1 token, not 12 bytes → 3 tokens.
        assert_eq!(estimate_tokens("日本語訳"), 1);
    }
}
