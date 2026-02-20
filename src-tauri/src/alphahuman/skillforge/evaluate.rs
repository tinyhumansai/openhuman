//! Evaluator - scores discovered skill candidates across multiple dimensions.

use serde::{Deserialize, Serialize};

use super::scout::ScoutResult;

// ---------------------------------------------------------------------------
// Scoring dimensions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scores {
    /// OS / arch / runtime compatibility (0.0-1.0).
    pub compatibility: f64,
    /// Code quality signals: stars, tests, docs (0.0-1.0).
    pub quality: f64,
    /// Security posture: license, known-bad patterns (0.0-1.0).
    pub security: f64,
}

impl Scores {
    /// Weighted total. Weights: compatibility 0.3, quality 0.35, security 0.35.
    pub fn total(&self) -> f64 {
        self.compatibility * 0.30 + self.quality * 0.35 + self.security * 0.35
    }
}

// ---------------------------------------------------------------------------
// Recommendation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Recommendation {
    /// Score >= threshold → safe to auto-integrate.
    Auto,
    /// Score in [0.4, threshold) → needs human review.
    Manual,
    /// Score < 0.4 → skip entirely.
    Skip,
}

// ---------------------------------------------------------------------------
// EvalResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub candidate: ScoutResult,
    pub scores: Scores,
    pub total_score: f64,
    pub recommendation: Recommendation,
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

pub struct Evaluator {
    /// Minimum total score for auto-integration.
    min_score: f64,
}

/// Known-bad patterns in repo names / descriptions (matched as whole words).
const BAD_PATTERNS: &[&str] = &[
    "malware",
    "exploit",
    "hack",
    "crack",
    "keygen",
    "ransomware",
    "trojan",
];

/// Check if `haystack` contains `word` as a whole word (bounded by non-alphanumeric chars).
fn contains_word(haystack: &str, word: &str) -> bool {
    for (i, _) in haystack.match_indices(word) {
        let before_ok = i == 0 || !haystack.as_bytes()[i - 1].is_ascii_alphanumeric();
        let after = i + word.len();
        let after_ok =
            after >= haystack.len() || !haystack.as_bytes()[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

impl Evaluator {
    pub fn new(min_score: f64) -> Self {
        Self { min_score }
    }

    pub fn evaluate(&self, candidate: ScoutResult) -> EvalResult {
        let compatibility = self.score_compatibility(&candidate);
        let quality = self.score_quality(&candidate);
        let security = self.score_security(&candidate);

        let scores = Scores {
            compatibility,
            quality,
            security,
        };
        let total_score = scores.total();

        let recommendation = if total_score >= self.min_score {
            Recommendation::Auto
        } else if total_score >= 0.4 {
            Recommendation::Manual
        } else {
            Recommendation::Skip
        };

        EvalResult {
            candidate,
            scores,
            total_score,
            recommendation,
        }
    }

    // -- Dimension scorers --------------------------------------------------

    /// Compatibility: favour Rust repos; penalise unknown languages.
    fn score_compatibility(&self, c: &ScoutResult) -> f64 {
        match c.language.as_deref() {
            Some("Rust") => 1.0,
            Some("Python" | "TypeScript" | "JavaScript") => 0.6,
            Some(_) => 0.3,
            None => 0.2,
        }
    }

    /// Quality: based on star count (log scale, capped at 1.0).
    fn score_quality(&self, c: &ScoutResult) -> f64 {
        // log2(stars + 1) / 10, capped at 1.0
        let raw = ((c.stars as f64) + 1.0).log2() / 10.0;
        raw.min(1.0)
    }

    /// Security: license presence + bad-pattern check.
    fn score_security(&self, c: &ScoutResult) -> f64 {
        let mut score: f64 = 0.5;

        // License bonus
        if c.has_license {
            score += 0.3;
        }

        // Bad-pattern penalty (whole-word match)
        let lower_name = c.name.to_lowercase();
        let lower_desc = c.description.to_lowercase();
        for pat in BAD_PATTERNS {
            if contains_word(&lower_name, pat) || contains_word(&lower_desc, pat) {
                score -= 0.5;
                break;
            }
        }

        // Recency bonus: updated within last 180 days (guard against future timestamps)
        if let Some(updated) = c.updated_at {
            let age_days = (chrono::Utc::now() - updated).num_days();
            if (0..180).contains(&age_days) {
                score += 0.2;
            }
        }

        score.clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphahuman::skillforge::scout::{ScoutResult, ScoutSource};

    fn make_candidate(stars: u64, lang: Option<&str>, has_license: bool) -> ScoutResult {
        ScoutResult {
            name: "test-skill".into(),
            url: "https://github.com/test/test-skill".into(),
            description: "A test skill".into(),
            stars,
            language: lang.map(String::from),
            updated_at: Some(chrono::Utc::now()),
            source: ScoutSource::GitHub,
            owner: "test".into(),
            has_license,
        }
    }

    #[test]
    fn high_quality_rust_repo_gets_auto() {
        let eval = Evaluator::new(0.7);
        let c = make_candidate(500, Some("Rust"), true);
        let res = eval.evaluate(c);
        assert!(res.total_score >= 0.7, "score: {}", res.total_score);
        assert_eq!(res.recommendation, Recommendation::Auto);
    }

    #[test]
    fn low_star_no_license_gets_manual_or_skip() {
        let eval = Evaluator::new(0.7);
        let c = make_candidate(1, None, false);
        let res = eval.evaluate(c);
        assert!(res.total_score < 0.7, "score: {}", res.total_score);
        assert_ne!(res.recommendation, Recommendation::Auto);
    }

    #[test]
    fn bad_pattern_tanks_security() {
        let eval = Evaluator::new(0.7);
        let mut c = make_candidate(1000, Some("Rust"), true);
        c.name = "malware-skill".into();
        let res = eval.evaluate(c);
        // 0.5 base + 0.3 license - 0.5 bad_pattern + 0.2 recency = 0.5
        assert!(
            res.scores.security <= 0.5,
            "security: {}",
            res.scores.security
        );
    }

    #[test]
    fn scores_total_weighted() {
        let s = Scores {
            compatibility: 1.0,
            quality: 1.0,
            security: 1.0,
        };
        assert!((s.total() - 1.0).abs() < f64::EPSILON);

        let s2 = Scores {
            compatibility: 0.0,
            quality: 0.0,
            security: 0.0,
        };
        assert!((s2.total()).abs() < f64::EPSILON);
    }

    #[test]
    fn hackathon_not_flagged_as_bad() {
        let eval = Evaluator::new(0.7);
        let mut c = make_candidate(500, Some("Rust"), true);
        c.name = "hackathon-tools".into();
        c.description = "Tools for hackathons and lifehacks".into();
        let res = eval.evaluate(c);
        // "hack" should NOT match "hackathon" or "lifehacks"
        assert!(
            res.scores.security >= 0.5,
            "security: {}",
            res.scores.security
        );
    }

    #[test]
    fn exact_hack_is_flagged() {
        let eval = Evaluator::new(0.7);
        let mut c = make_candidate(500, Some("Rust"), false);
        c.name = "hack-tool".into();
        c.updated_at = None;
        let res = eval.evaluate(c);
        // 0.5 base + 0.0 license - 0.5 bad_pattern + 0.0 recency = 0.0
        assert!(
            res.scores.security < 0.5,
            "security: {}",
            res.scores.security
        );
    }
}
