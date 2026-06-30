//! Shared importance signals for the content-router compressors.
//!
//! A small, deterministic keyword registry + per-line scorer used by the
//! search, log, and JSON compressors to decide which lines/rows to keep when a
//! tool output is over budget. No ML, no regex on the hot path beyond simple
//! case-insensitive substring scans.
//!
//! Clean-room port of Headroom's `error_detection` priority signals
//! (Apache-2.0): error/fatal lines score highest, warnings next, importance
//! markers (security/TODO) a small bump, everything else baseline.

/// Keywords that mark a hard failure (case-insensitive substrings).
const ERROR_KEYWORDS: &[&str] = &[
    "error",
    "fatal",
    "panic",
    "panicked",
    "exception",
    "traceback",
    "failed",
    "failure",
    "segfault",
    "assertion",
    "abort",
    "[error]",
    "error:",
];

/// Keywords that mark a warning. Lower weight than errors.
const WARNING_KEYWORDS: &[&str] = &["warning", "warn:", "[warn]", "deprecated"];

/// Keywords that bump importance regardless of severity.
const IMPORTANCE_KEYWORDS: &[&str] = &[
    "security",
    "vulnerability",
    "critical",
    "todo",
    "fixme",
    "denied",
    "unauthorized",
    "forbidden",
];

/// Score weights. Higher = more likely to survive truncation.
pub const SCORE_ERROR: f32 = 1.0;
pub const SCORE_WARNING: f32 = 0.6;
pub const SCORE_IMPORTANCE: f32 = 0.4;
pub const SCORE_BASELINE: f32 = 0.1;

/// True if any error keyword appears in `text` (case-insensitive).
pub fn has_error_indicators(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ERROR_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Importance score for a single line in `[0.0, 1.0]`.
pub fn line_score(line: &str) -> f32 {
    let lower = line.to_ascii_lowercase();
    let mut score = SCORE_BASELINE;
    if ERROR_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        score = score.max(SCORE_ERROR);
    }
    if WARNING_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        score = score.max(SCORE_WARNING);
    }
    if IMPORTANCE_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        score = score.max(SCORE_IMPORTANCE);
    }
    score
}

/// Classify a line's severity for the log compressor's bucketing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Other,
}

/// Bucket a line into [`Severity`].
pub fn severity(line: &str) -> Severity {
    let lower = line.to_ascii_lowercase();
    if ERROR_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        Severity::Error
    } else if WARNING_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        Severity::Warning
    } else {
        Severity::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_score_highest() {
        assert_eq!(line_score("FATAL: connection refused"), SCORE_ERROR);
        assert_eq!(line_score("thread panicked at 'boom'"), SCORE_ERROR);
        assert!(line_score("error: mismatched types") >= SCORE_ERROR);
    }

    #[test]
    fn warnings_below_errors_above_baseline() {
        let w = line_score("warning: unused variable");
        assert!(w < SCORE_ERROR);
        assert!(w > SCORE_BASELINE);
    }

    #[test]
    fn plain_line_is_baseline() {
        assert_eq!(line_score("   Compiling foo v0.1.0"), SCORE_BASELINE);
    }

    #[test]
    fn severity_buckets() {
        assert_eq!(severity("error[E0382]: borrow"), Severity::Error);
        assert_eq!(severity("warning: deprecated"), Severity::Warning);
        assert_eq!(severity("running 12 tests"), Severity::Other);
    }

    #[test]
    fn has_error_indicators_detects() {
        assert!(has_error_indicators("test result: FAILED"));
        assert!(!has_error_indicators("all good, 12 passed"));
    }
}
