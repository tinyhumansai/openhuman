//! Shared importance signals for the compaction compressors.
//!
//! A small, deterministic keyword registry + per-line scorer used by
//! [`super::search`] and [`super::log`] to decide which lines to keep when a
//! tool output is over budget. No ML, no regex compilation cost on the hot
//! path beyond simple case-insensitive substring scans.
//!
//! Behavior is a clean-room port of headroom's `error_detection` priority
//! signals (Apache-2.0): error/fatal lines score highest, warnings next,
//! importance markers (security/TODO) a small bump, everything else baseline.

/// Keywords that mark a hard failure. Matched case-insensitively as
/// substrings. Kept deliberately small and high-precision — a false positive
/// just means we keep a line we could have dropped, which is the safe
/// direction.
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

/// Keywords that bump importance regardless of severity — things an agent
/// almost always wants to see even in a truncated view.
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

/// True if any error keyword appears in `text` (case-insensitive). Cheap
/// pre-check used to decide whether a blob is worth the log compressor.
pub fn has_error_indicators(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ERROR_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Importance score for a single line in `[0.0, 1.0]`. Errors dominate,
/// then warnings, then importance markers; a plain line gets the baseline so
/// ordering is stable and "keep highest N" never discards everything.
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

/// Bucket a line into [`Severity`]. Used by [`super::log`] to keep error and
/// warning lines under separate caps.
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
    fn importance_markers_bump() {
        assert!(line_score("TODO: handle retry") > SCORE_BASELINE);
        assert!(line_score("potential security issue here") > SCORE_BASELINE);
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
