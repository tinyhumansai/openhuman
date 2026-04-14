//! Whisper hallucination detection — shared filter for all voice pipelines.
//!
//! Whisper.cpp outputs "[BLANK_AUDIO]" for silence and stock phrases
//! ("Thank you for watching", etc.) when fed noisy or near-empty audio.
//! This module provides a robust detector that catches:
//!
//! - Exact-match known hallucination phrases
//! - Uniform single-word repetition ("you you you you")
//! - Punctuation-variant repetition ("it... it... it...")
//! - Ratio-based repetition (any single word > 40% of total words)

use log::debug;

const LOG_PREFIX: &str = "[voice][hallucination]";

/// Known whisper hallucination patterns. These are common outputs when
/// whisper processes near-silent audio or audio with background noise.
/// Sourced from community lists and OpenWhispr's filtering behavior.
pub const HALLUCINATION_PATTERNS: &[&str] = &[
    // whisper.cpp blank markers
    "[blank_audio]",
    "[ blank_audio ]",
    "[blank audio]",
    "(blank audio)",
    // Common hallucinations from YouTube-trained models
    "thank you",
    "thank you.",
    "thanks.",
    "thank you for watching",
    "thanks for watching",
    "thank you for listening",
    "thanks for listening",
    "thank you so much",
    "please subscribe",
    "like and subscribe",
    "see you next time",
    "see you in the next video",
    "bye bye",
    "bye.",
    "goodbye.",
    // Single-word noise artifacts
    "you",
    "the",
    "i",
    "a",
    "so",
    "okay",
    "ok",
    "yeah",
    "yes",
    "no",
    "oh",
    "hmm",
    "huh",
    "ah",
    // Punctuation-only
    "...",
    ".",
    ",",
    "!",
    "?",
];

/// Strip all ASCII punctuation from a word, returning the bare alphabetic core.
fn strip_punctuation(word: &str) -> String {
    word.chars().filter(|c| !c.is_ascii_punctuation()).collect()
}

/// Check if whisper output is a known hallucination pattern.
///
/// Detection layers (applied in order):
/// 1. **Exact match** against `HALLUCINATION_PATTERNS` (with trailing-punctuation stripping).
/// 2. **Uniform repetition** — all words are the same after punctuation stripping
///    (catches "it... it... it..." and "you you you you").
/// 3. **Dominant-word ratio** — any single word comprising > 40% of total words
///    (catches mixed hallucination like "thank you thank you thank you hello").
pub fn is_hallucinated_output(text: &str) -> bool {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return false; // handled separately as "empty"
    }

    // Strip trailing punctuation for matching (whisper often appends periods).
    let stripped = normalized.trim_end_matches(|c: char| c.is_ascii_punctuation());

    // Layer 1: Exact match against known hallucination phrases.
    for pattern in HALLUCINATION_PATTERNS {
        if normalized == *pattern || stripped == *pattern {
            debug!(
                "{LOG_PREFIX} exact-match hallucination detected: {:?}",
                normalized
            );
            return true;
        }
    }

    // Tokenize into words, stripping punctuation from each for comparison.
    let raw_words: Vec<&str> = normalized.split_whitespace().collect();
    if raw_words.len() < 3 {
        return false;
    }

    let clean_words: Vec<String> = raw_words
        .iter()
        .map(|w| strip_punctuation(w))
        .filter(|w| !w.is_empty())
        .collect();

    if clean_words.is_empty() {
        return false;
    }

    // Layer 2: Uniform repetition — all cleaned words identical.
    let first = &clean_words[0];
    if clean_words.iter().all(|w| w == first) {
        debug!(
            "{LOG_PREFIX} uniform repetition detected: {:?} ({} repeats)",
            first,
            clean_words.len()
        );
        return true;
    }

    // Layer 3: Dominant-word ratio — any word > 40% of total is suspicious.
    let total = clean_words.len();
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for w in &clean_words {
        *counts.entry(w.as_str()).or_insert(0) += 1;
    }
    for (word, count) in &counts {
        let ratio = *count as f64 / total as f64;
        if ratio > 0.4 && *count >= 3 {
            debug!(
                "{LOG_PREFIX} dominant-word hallucination: {:?} appears {}/{} times ({:.0}%)",
                word,
                count,
                total,
                ratio * 100.0
            );
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Exact-match hallucinations ---

    #[test]
    fn exact_match_thank_you() {
        assert!(is_hallucinated_output("Thank you."));
        assert!(is_hallucinated_output("thank you"));
        assert!(is_hallucinated_output("THANK YOU"));
    }

    #[test]
    fn exact_match_blank_audio() {
        assert!(is_hallucinated_output("[blank_audio]"));
        assert!(is_hallucinated_output("[ blank_audio ]"));
    }

    #[test]
    fn exact_match_single_word() {
        assert!(is_hallucinated_output("you"));
        assert!(is_hallucinated_output("the"));
        assert!(is_hallucinated_output("hmm"));
    }

    #[test]
    fn exact_match_punctuation_only() {
        assert!(is_hallucinated_output("..."));
        assert!(is_hallucinated_output("."));
    }

    // --- Uniform repetition ---

    #[test]
    fn uniform_repetition_plain() {
        assert!(is_hallucinated_output("you you you you"));
        assert!(is_hallucinated_output("the the the the the"));
    }

    #[test]
    fn uniform_repetition_with_punctuation() {
        // The key bug this fixes — "it... it... it..." was NOT caught before
        assert!(is_hallucinated_output("it... it... it..."));
        assert!(is_hallucinated_output("it, it, it, it"));
        assert!(is_hallucinated_output("Thank you. Thank you. Thank you."));
    }

    // --- Dominant-word ratio ---

    #[test]
    fn dominant_word_mixed() {
        // "thank" appears 3/6 = 50% > 40%
        assert!(is_hallucinated_output(
            "thank you thank you thank you hello"
        ));
    }

    #[test]
    fn dominant_word_it_pattern() {
        // Reproduces the teammate screenshot: massive "it" repetition
        assert!(is_hallucinated_output(
            "it it it it it it it it hello world"
        ));
    }

    // --- Non-hallucinations (should NOT be flagged) ---

    #[test]
    fn legitimate_short_sentence() {
        assert!(!is_hallucinated_output(
            "Can you check the latest price of Bitcoin?"
        ));
    }

    #[test]
    fn legitimate_with_repeated_common_word() {
        // "the" appears 2/8 = 25% < 40%, so not flagged
        assert!(!is_hallucinated_output(
            "I went to the store and the park today"
        ));
    }

    #[test]
    fn empty_string() {
        assert!(!is_hallucinated_output(""));
        assert!(!is_hallucinated_output("   "));
    }

    #[test]
    fn two_word_input_not_flagged() {
        // Below the 3-word minimum for repetition checks
        assert!(!is_hallucinated_output("hello world"));
    }

    #[test]
    fn legitimate_conversation() {
        assert!(!is_hallucinated_output(
            "Hey team, let's discuss the new feature implementation plan for next sprint"
        ));
    }
}
