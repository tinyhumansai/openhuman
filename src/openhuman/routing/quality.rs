//! Response quality assessment for routing fallback decisions.
//!
//! When the local model returns a response, these heuristics determine whether
//! it is good enough to serve to the caller or whether a remote fallback should
//! be triggered. The checks are intentionally simple and fast — they run on the
//! hot path before a potential second inference call.

/// Minimum character count for a response to be considered non-trivial.
const MIN_CHARS: usize = 5;

/// Returns `true` when `text` should be treated as low quality and a remote
/// fallback is warranted.
///
/// Heuristics (all fast, no I/O):
/// - Empty or shorter than [`MIN_CHARS`] after trimming.
/// - Starts with a known model refusal / inability phrase.
///
/// These patterns are deliberately conservative: a false positive (falling back
/// unnecessarily) is cheaper than a false negative (serving a bad response).
pub fn is_low_quality(text: &str) -> bool {
    let trimmed = text.trim();

    if trimmed.len() < MIN_CHARS {
        return true;
    }

    // Common refusal/inability phrases from small local models.
    let lower = trimmed.to_lowercase();
    REFUSAL_PREFIXES.iter().any(|p| lower.starts_with(p))
}

const REFUSAL_PREFIXES: &[&str] = &[
    "i cannot",
    "i can't",
    "i'm unable to",
    "i am unable to",
    "as an ai,",
    "as an ai language",
    "i don't have the ability",
    "i'm sorry, but i cannot",
    "i apologize, but i cannot",
    "sorry, i cannot",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_low_quality() {
        assert!(is_low_quality(""));
        assert!(is_low_quality("   "));
    }

    #[test]
    fn too_short_is_low_quality() {
        assert!(is_low_quality("ok"));
        assert!(is_low_quality("yes"));
        assert!(is_low_quality("no"));
    }

    #[test]
    fn normal_response_is_not_low_quality() {
        assert!(!is_low_quality("The answer is 42."));
        assert!(!is_low_quality("Here is a summary of the article."));
    }

    #[test]
    fn refusal_prefixes_are_low_quality() {
        assert!(is_low_quality("I cannot help with that."));
        assert!(is_low_quality("I can't do that."));
        assert!(is_low_quality("I'm unable to process this request."));
        assert!(is_low_quality("I am unable to assist."));
        assert!(is_low_quality("As an AI, I don't have opinions."));
        assert!(is_low_quality("As an AI language model, I cannot..."));
        assert!(is_low_quality(
            "I don't have the ability to browse the web."
        ));
        assert!(is_low_quality("I'm sorry, but I cannot comply."));
        assert!(is_low_quality("I apologize, but I cannot do that."));
        assert!(is_low_quality("Sorry, I cannot assist with that."));
    }

    #[test]
    fn refusal_check_is_case_insensitive() {
        assert!(is_low_quality("I CANNOT help with that."));
        assert!(is_low_quality("I CAN'T do that."));
    }

    #[test]
    fn borderline_length_not_flagged_if_content_ok() {
        // Exactly 5 chars — not low quality by length alone.
        assert!(!is_low_quality("Hello"));
        // 4 chars — below threshold.
        assert!(is_low_quality("Hi!"));
    }
}
