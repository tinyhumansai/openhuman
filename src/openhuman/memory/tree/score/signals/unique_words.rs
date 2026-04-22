//! Unique-word-ratio signal — noise detector that fires on low-diversity text.
//!
//! Example: "yay yay yay yay lol lol lol" has high repetition = low diversity.
//! A substantive message has high type-token ratio (roughly, unique words /
//! total words).
//!
//! For very short messages the ratio is naturally ~1.0, so we require a
//! minimum total count before this signal contributes — otherwise "hi bob"
//! would score identically to a real message.

pub const MIN_TOTAL_WORDS: usize = 5;

/// Score in `[0.0, 1.0]` from the type-token ratio of `text`.
///
/// - Too few total words → `0.5` (indeterminate — defer to other signals)
/// - Ratio < 0.3 (heavy repetition) → 0.0
/// - Ratio >= 0.7 (substantive) → 1.0
/// - Linear in between
pub fn score(text: &str) -> f32 {
    let mut total: usize = 0;
    let mut uniq: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for raw in text.split_whitespace() {
        let w: String = raw
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if w.is_empty() {
            continue;
        }
        total += 1;
        uniq.insert(w);
    }

    if total < MIN_TOTAL_WORDS {
        return 0.5;
    }

    let ratio = uniq.len() as f32 / total as f32;
    if ratio <= 0.3 {
        0.0
    } else if ratio >= 0.7 {
        1.0
    } else {
        (ratio - 0.3) / 0.4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_returns_neutral() {
        assert_eq!(score(""), 0.5);
        assert_eq!(score("hi bob"), 0.5);
    }

    #[test]
    fn high_repetition_scored_low() {
        let noisy = "yay yay yay yay yay yay yay yay yay yay lol lol lol lol";
        assert!(score(noisy) < 0.2);
    }

    #[test]
    fn substantive_text_scored_high() {
        let good =
            "We decided to ship Phoenix on Friday after reviewing the migration plan carefully.";
        assert!(score(good) >= 0.9);
    }

    #[test]
    fn medium_repetition_ramps() {
        // ~50% unique ratio should score around 0.5
        let med = "alpha beta alpha beta gamma alpha delta beta gamma alpha";
        let s = score(med);
        assert!(s > 0.2 && s < 0.8);
    }

    #[test]
    fn punctuation_stripped() {
        let s1 = score("ship phoenix friday ship phoenix friday ship phoenix");
        let s2 = score("ship! phoenix, friday. ship! phoenix, friday. ship! phoenix.");
        assert!((s1 - s2).abs() < 0.05);
    }
}
