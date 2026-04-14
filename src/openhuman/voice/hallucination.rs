//! Whisper hallucination detection — shared filter for all voice pipelines.
//!
//! Whisper.cpp outputs "[BLANK_AUDIO]" for silence and stock phrases
//! ("Thank you for watching", etc.) when fed noisy or near-empty audio.
//! This module provides a robust detector that catches:
//!
//! - Exact-match known hallucination phrases
//! - Uniform single-word repetition ("you you you you")
//! - Punctuation-variant repetition ("it... it... it...")
//! - Ratio-based repetition (any single word > 60% of total words)
//!
//! Two modes are supported via [`HallucinationMode`]:
//! - **Dictation** — aggressive filtering (single-word noise artifacts like
//!   "yes", "no", "okay" are dropped since they're almost certainly hallucination
//!   in a push-to-talk dictation context).
//! - **Conversation** — conservative filtering (short conversational replies
//!   like "yes", "okay", "thank you" are allowed through since they're
//!   legitimate chat responses).

use log::debug;

const LOG_PREFIX: &str = "[voice][hallucination]";

/// Controls how aggressively the hallucination filter operates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HallucinationMode {
    /// Desktop dictation (push-to-talk). Aggressive: single-word noise
    /// artifacts and short conversational phrases are treated as hallucination.
    Dictation,
    /// Chat voice input. Conservative: only blank-audio markers, YouTube
    /// hallucinations, and repetition patterns are filtered. Short
    /// conversational utterances like "yes" or "okay" pass through.
    Conversation,
}

/// Blank-audio markers and YouTube-trained hallucination phrases.
/// These are filtered in ALL modes — they are never legitimate speech.
const ALWAYS_HALLUCINATION: &[&str] = &[
    // whisper.cpp blank markers
    "[blank_audio]",
    "[ blank_audio ]",
    "[blank audio]",
    "(blank audio)",
    // Common hallucinations from YouTube-trained models
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
    // Punctuation-only
    "...",
    ".",
    ",",
    "!",
    "?",
];

/// Single-word noise artifacts and short phrases that are hallucination
/// in dictation mode but may be valid in conversation mode.
const DICTATION_ONLY_PATTERNS: &[&str] = &[
    "thank you",
    "thank you.",
    "thanks.",
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
];

/// Strip all ASCII punctuation from a word, returning the bare alphabetic core.
fn strip_punctuation(word: &str) -> String {
    word.chars().filter(|c| !c.is_ascii_punctuation()).collect()
}

/// Check if whisper output is a known hallucination pattern.
///
/// Detection layers (applied in order):
/// 1. **Exact match** against `ALWAYS_HALLUCINATION` patterns (both modes),
///    plus `DICTATION_ONLY_PATTERNS` when in dictation mode.
/// 2. **Uniform repetition** — all words are the same after punctuation stripping
///    (catches "it... it... it..." and "you you you you").
/// 3. **Dominant-word ratio** — any single word comprising > 60% of total words
///    with at least 5 occurrences (catches massive hallucination loops while
///    allowing natural emphatic phrases like "no no no don't do that").
pub fn is_hallucinated_output(text: &str, mode: HallucinationMode) -> bool {
    let normalized = text.trim().to_lowercase();
    if normalized.is_empty() {
        return false; // handled separately as "empty"
    }

    // Strip trailing punctuation for matching (whisper often appends periods).
    let stripped = normalized.trim_end_matches(|c: char| c.is_ascii_punctuation());

    // Layer 1: Exact match against known hallucination phrases.
    for pattern in ALWAYS_HALLUCINATION {
        if normalized == *pattern || stripped == *pattern {
            debug!(
                "{LOG_PREFIX} exact-match hallucination detected: {:?}",
                normalized
            );
            return true;
        }
    }

    // In dictation mode, also check the aggressive single-word/short-phrase list.
    if mode == HallucinationMode::Dictation {
        for pattern in DICTATION_ONLY_PATTERNS {
            if normalized == *pattern || stripped == *pattern {
                debug!(
                    "{LOG_PREFIX} dictation-only hallucination detected: {:?}",
                    normalized
                );
                return true;
            }
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

    // Layer 2b: Repeating n-gram — the entire utterance is a small phrase
    // (1-3 words) repeated multiple times. Catches "Thank you. Thank you.
    // Thank you." where no single word dominates but the phrase loops.
    for ngram_len in 1..=3 {
        if clean_words.len() >= ngram_len * 2 && clean_words.len().is_multiple_of(ngram_len) {
            let pattern = &clean_words[..ngram_len];
            let all_match = clean_words.chunks(ngram_len).all(|chunk| chunk == pattern);
            if all_match {
                debug!(
                    "{LOG_PREFIX} repeating {}-gram detected: {:?} ({} repeats)",
                    ngram_len,
                    pattern,
                    clean_words.len() / ngram_len
                );
                return true;
            }
        }
    }

    // Layer 3: Dominant-word ratio — any word > 60% of total with at least
    // 5 occurrences. This is conservative enough to allow emphatic phrases
    // like "no no no don't do that" (3/6 = 50%) while catching hallucination
    // loops like "it it it it it it it it hello world" (8/10 = 80%).
    let total = clean_words.len();
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for w in &clean_words {
        *counts.entry(w.as_str()).or_insert(0) += 1;
    }
    for (word, count) in &counts {
        let ratio = *count as f64 / total as f64;
        if ratio > 0.6 && *count >= 5 {
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

    // --- Exact-match hallucinations (both modes) ---

    #[test]
    fn exact_match_blank_audio() {
        assert!(is_hallucinated_output(
            "[blank_audio]",
            HallucinationMode::Conversation
        ));
        assert!(is_hallucinated_output(
            "[ blank_audio ]",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn exact_match_youtube_hallucination() {
        assert!(is_hallucinated_output(
            "Thank you for watching",
            HallucinationMode::Conversation
        ));
        assert!(is_hallucinated_output(
            "please subscribe",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn exact_match_punctuation_only() {
        assert!(is_hallucinated_output(
            "...",
            HallucinationMode::Conversation
        ));
        assert!(is_hallucinated_output(".", HallucinationMode::Conversation));
    }

    // --- Dictation-only patterns ---

    #[test]
    fn dictation_mode_drops_single_words() {
        assert!(is_hallucinated_output("you", HallucinationMode::Dictation));
        assert!(is_hallucinated_output("okay", HallucinationMode::Dictation));
        assert!(is_hallucinated_output(
            "Thank you.",
            HallucinationMode::Dictation
        ));
        assert!(is_hallucinated_output("yes", HallucinationMode::Dictation));
    }

    #[test]
    fn conversation_mode_allows_short_replies() {
        // These are valid chat responses — should NOT be filtered in conversation mode.
        assert!(!is_hallucinated_output(
            "yes",
            HallucinationMode::Conversation
        ));
        assert!(!is_hallucinated_output(
            "no",
            HallucinationMode::Conversation
        ));
        assert!(!is_hallucinated_output(
            "okay",
            HallucinationMode::Conversation
        ));
        assert!(!is_hallucinated_output(
            "thank you",
            HallucinationMode::Conversation
        ));
        assert!(!is_hallucinated_output(
            "goodbye",
            HallucinationMode::Conversation
        ));
    }

    // --- Uniform repetition (both modes) ---

    #[test]
    fn uniform_repetition_plain() {
        assert!(is_hallucinated_output(
            "you you you you",
            HallucinationMode::Conversation
        ));
        assert!(is_hallucinated_output(
            "the the the the the",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn uniform_repetition_with_punctuation() {
        assert!(is_hallucinated_output(
            "it... it... it...",
            HallucinationMode::Conversation
        ));
        assert!(is_hallucinated_output(
            "it, it, it, it",
            HallucinationMode::Conversation
        ));
        assert!(is_hallucinated_output(
            "Thank you. Thank you. Thank you.",
            HallucinationMode::Conversation
        ));
    }

    // --- Dominant-word ratio (stricter thresholds) ---

    #[test]
    fn dominant_word_massive_repetition() {
        // "it" appears 8/10 = 80% with count=8 >= 5 — flagged
        assert!(is_hallucinated_output(
            "it it it it it it it it hello world",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn emphatic_phrase_not_flagged() {
        // "no" appears 3/6 = 50% with count=3 < 5 — NOT flagged (natural speech)
        assert!(!is_hallucinated_output(
            "no no no don't do that",
            HallucinationMode::Conversation
        ));
        // "go" appears 3/5 = 60% with count=3 < 5 — NOT flagged
        assert!(!is_hallucinated_output(
            "go go go turn left",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn moderate_repetition_not_flagged() {
        // "thank" appears 3/7 = 43% — below 60%, NOT flagged
        assert!(!is_hallucinated_output(
            "thank you thank you thank you hello",
            HallucinationMode::Conversation
        ));
    }

    // --- Non-hallucinations (should NOT be flagged) ---

    #[test]
    fn legitimate_short_sentence() {
        assert!(!is_hallucinated_output(
            "Can you check the latest price of Bitcoin?",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn legitimate_with_repeated_common_word() {
        assert!(!is_hallucinated_output(
            "I went to the store and the park today",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn empty_string() {
        assert!(!is_hallucinated_output("", HallucinationMode::Conversation));
        assert!(!is_hallucinated_output(
            "   ",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn two_word_input_not_flagged() {
        assert!(!is_hallucinated_output(
            "hello world",
            HallucinationMode::Conversation
        ));
    }

    #[test]
    fn legitimate_conversation() {
        assert!(!is_hallucinated_output(
            "Hey team, let's discuss the new feature implementation plan for next sprint",
            HallucinationMode::Conversation
        ));
    }
}
