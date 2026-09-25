//! Public result types for a triage run: which arm produced the
//! decision, the decision itself, and the deferral variant.

use super::super::decision::TriageDecision;

/// Which arm produced this triage decision. Surfaced on `TriageRun`
/// so the orchestrator can colour-code degraded turns and show the
/// state in `/debug` views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageResolutionPath {
    /// Cloud succeeded on the initial attempt.
    Cloud,
    /// Cloud succeeded on the retry after a 429 / transient failure.
    CloudAfterRetry,
    /// Cloud failed twice; the local arm produced the decision.
    LocalFallback,
}

impl TriageResolutionPath {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::CloudAfterRetry => "cloud-after-retry",
            Self::LocalFallback => "local-fallback",
        }
    }
}

/// Final output of a single triage run when a decision was produced.
#[derive(Debug, Clone)]
pub struct TriageRun {
    pub decision: TriageDecision,
    /// `true` when the producing arm was local — kept for telemetry
    /// compatibility with subscribers that read this field. Equivalent
    /// to `resolution_path == LocalFallback`.
    pub used_local: bool,
    pub latency_ms: u64,
    pub resolution_path: TriageResolutionPath,
}

/// Outcome of [`run_triage`]. Either a parsed decision, a retryable
/// deferral, or a terminal outcome when no fallback exists.
#[derive(Debug, Clone)]
pub enum TriageOutcome {
    Decision(TriageRun),
    Deferred {
        /// Unix epoch millis at which the caller should re-run the
        /// triage chain.
        defer_until_ms: i64,
        /// Short human-readable reason — already scrubbed; safe to log.
        reason: String,
    },
    /// No local fallback is configured and the cloud retry budget is
    /// exhausted. Callers must record this state instead of scheduling
    /// another fixed-interval model invocation.
    Terminal {
        reason: String,
    },
}

impl TriageOutcome {
    pub fn into_decision(self) -> Option<TriageRun> {
        match self {
            TriageOutcome::Decision(run) => Some(run),
            TriageOutcome::Deferred { .. } | TriageOutcome::Terminal { .. } => None,
        }
    }
}
