//! Classifier entry point for the local PII detector.
//!
//! [`scan`] is a pure, synchronous function: it walks the compiled rule set
//! (see [`super::rules`]) over the input, tallies matches per category, and
//! folds the tally into a [`PiiScanResult`]. No async, no I/O, no network.

use std::collections::BTreeMap;

use super::rules::rules;
use super::types::{CategoryHit, PiiCategory, PiiScanResult, RiskLevel};

/// Extra points added per additional co-occurring category. Multiple distinct
/// identifier/context categories in one blob look more like a real record than
/// any single one alone, so co-occurrence nudges the score upward.
const CO_OCCURRENCE_BONUS: u32 = 5;

/// Map a raw score to a [`RiskLevel`]. Thresholds lean toward escalation
/// (recall over precision).
fn level_from_score(score: u32) -> RiskLevel {
    match score {
        0 => RiskLevel::None,
        1..=19 => RiskLevel::Low,
        20..=44 => RiskLevel::Medium,
        _ => RiskLevel::High,
    }
}

/// Scan `content` for identification risk.
///
/// Returns a [`PiiScanResult`] carrying the overall [`RiskLevel`], a numeric
/// score, and the distinct categories detected (with per-category counts).
///
/// Detection runs **fully locally** — pattern + keyword matching only. The
/// function is deterministic and side-effect-free, so it is safe to call from
/// any context (sync or async, on or off a Tokio runtime).
///
/// ## Scoring
/// * Each distinct category contributes its [`PiiCategory::weight`] once.
/// * Each additional co-occurring category adds [`CO_OCCURRENCE_BONUS`].
/// * A "strong identifier" (SSN, card, passport, bank account) forces
///   [`RiskLevel::High`] regardless of the numeric score.
pub fn scan(content: &str) -> PiiScanResult {
    // BTreeMap keeps the output deterministic (categories emitted in enum
    // order) without a separate sort pass.
    let mut counts: BTreeMap<PiiCategory, usize> = BTreeMap::new();

    for rule in rules() {
        for m in rule.regex.find_iter(content) {
            if let Some(validator) = rule.validator {
                if !validator(m.as_str()) {
                    continue;
                }
            }
            *counts.entry(rule.category).or_default() += 1;
        }
    }

    if counts.is_empty() {
        log::trace!("[security][pii] scan clean (no categories matched)");
        return PiiScanResult::default();
    }

    let distinct = counts.len() as u32;
    let mut score: u32 = counts.keys().map(|c| c.weight()).sum();
    if distinct > 1 {
        score += CO_OCCURRENCE_BONUS * (distinct - 1);
    }

    let has_strong = counts.keys().any(|c| c.is_strong_identifier());
    let mut level = level_from_score(score);
    if has_strong && level < RiskLevel::High {
        level = RiskLevel::High;
    }

    let categories: Vec<PiiCategory> = counts.keys().copied().collect();
    let hits: Vec<CategoryHit> = counts
        .iter()
        .map(|(&category, &count)| CategoryHit { category, count })
        .collect();

    // Log level/score/categories only — never the matched content itself.
    log::debug!(
        "[security][pii] scan level={} score={} categories={:?}",
        level.as_str(),
        score,
        categories.iter().map(|c| c.as_str()).collect::<Vec<_>>()
    );

    PiiScanResult {
        level,
        score,
        categories,
        hits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_none() {
        let r = scan("");
        assert_eq!(r.level, RiskLevel::None);
        assert_eq!(r.score, 0);
        assert!(r.categories.is_empty());
        assert!(!r.is_sensitive());
    }

    #[test]
    fn level_thresholds_are_monotonic() {
        assert_eq!(level_from_score(0), RiskLevel::None);
        assert_eq!(level_from_score(1), RiskLevel::Low);
        assert_eq!(level_from_score(19), RiskLevel::Low);
        assert_eq!(level_from_score(20), RiskLevel::Medium);
        assert_eq!(level_from_score(44), RiskLevel::Medium);
        assert_eq!(level_from_score(45), RiskLevel::High);
        assert_eq!(level_from_score(1000), RiskLevel::High);
    }
}
