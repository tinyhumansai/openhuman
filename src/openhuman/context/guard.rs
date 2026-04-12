//! Pre-inference context window guard with compaction circuit breaker.
//!
//! Checks context utilization before each LLM call and triggers auto-compaction
//! when usage exceeds a threshold. A circuit breaker disables compaction after
//! consecutive failures to prevent infinite retry loops.

use crate::openhuman::providers::UsageInfo;

/// Threshold (0.0–1.0) at which auto-compaction is triggered.
pub(crate) const COMPACTION_TRIGGER_THRESHOLD: f64 = 0.90;

/// Threshold above which, if compaction is disabled, the guard returns an error.
const HARD_LIMIT_THRESHOLD: f64 = 0.95;

/// Number of consecutive compaction failures before the circuit breaker trips.
const MAX_CONSECUTIVE_FAILURES: u8 = 3;

/// Outcome of a pre-inference context check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextCheckResult {
    /// Context utilization is within safe limits.
    Ok,
    /// Context is near capacity; compaction should be attempted.
    CompactionNeeded,
    /// Context is critically full and compaction is disabled (circuit breaker tripped).
    ContextExhausted { utilization_pct: u8, reason: String },
}

/// Tracks context window utilization and compaction health.
#[derive(Debug)]
pub struct ContextGuard {
    /// Last known input token count from the provider.
    last_input_tokens: u64,
    /// Last known output token count from the provider.
    last_output_tokens: u64,
    /// Model context window size (0 = unknown, guard is a no-op).
    context_window: u64,
    /// Number of consecutive compaction failures.
    consecutive_compaction_failures: u8,
    /// Whether compaction has been disabled by the circuit breaker.
    compaction_disabled: bool,
}

impl Default for ContextGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextGuard {
    pub fn new() -> Self {
        Self {
            last_input_tokens: 0,
            last_output_tokens: 0,
            context_window: 0,
            consecutive_compaction_failures: 0,
            compaction_disabled: false,
        }
    }

    /// Create a guard with a known context window size.
    pub fn with_context_window(context_window: u64) -> Self {
        Self {
            context_window,
            ..Self::new()
        }
    }

    /// Update the guard with usage info from the latest provider response.
    pub fn update_usage(&mut self, usage: &UsageInfo) {
        self.last_input_tokens = usage.input_tokens;
        self.last_output_tokens = usage.output_tokens;
        if usage.context_window > 0 {
            self.context_window = usage.context_window;
        }
    }

    /// Estimate current context utilization as a fraction (0.0–1.0).
    /// Returns `None` if context window is unknown.
    pub fn utilization(&self) -> Option<f64> {
        if self.context_window == 0 {
            return None;
        }
        let total_used = self.last_input_tokens + self.last_output_tokens;
        Some(total_used as f64 / self.context_window as f64)
    }

    /// Check whether the context is safe to proceed with another inference call.
    pub fn check(&self) -> ContextCheckResult {
        let utilization = match self.utilization() {
            Some(u) => u,
            None => return ContextCheckResult::Ok, // Unknown window = no guard
        };

        if utilization >= HARD_LIMIT_THRESHOLD && self.compaction_disabled {
            return ContextCheckResult::ContextExhausted {
                utilization_pct: (utilization * 100.0) as u8,
                reason: format!(
                    "Context {:.0}% full; compaction disabled after {} consecutive failures",
                    utilization * 100.0,
                    self.consecutive_compaction_failures
                ),
            };
        }

        if utilization >= COMPACTION_TRIGGER_THRESHOLD && !self.compaction_disabled {
            return ContextCheckResult::CompactionNeeded;
        }

        ContextCheckResult::Ok
    }

    /// Record a successful compaction, resetting the failure counter.
    pub fn record_compaction_success(&mut self) {
        self.consecutive_compaction_failures = 0;
        self.compaction_disabled = false;
        tracing::debug!("[context_guard] compaction succeeded, circuit breaker reset");
    }

    /// Record a failed compaction attempt. Trips the circuit breaker after
    /// `MAX_CONSECUTIVE_FAILURES` failures.
    pub fn record_compaction_failure(&mut self) {
        self.consecutive_compaction_failures += 1;
        if self.consecutive_compaction_failures >= MAX_CONSECUTIVE_FAILURES {
            self.compaction_disabled = true;
            tracing::warn!(
                consecutive_failures = self.consecutive_compaction_failures,
                "[context_guard] circuit breaker tripped — compaction disabled"
            );
        } else {
            tracing::debug!(
                consecutive_failures = self.consecutive_compaction_failures,
                max = MAX_CONSECUTIVE_FAILURES,
                "[context_guard] compaction failed, circuit breaker pending"
            );
        }
    }

    /// Whether the compaction circuit breaker is currently tripped.
    pub fn is_compaction_disabled(&self) -> bool {
        self.compaction_disabled
    }

    /// Number of consecutive compaction failures.
    pub fn consecutive_failures(&self) -> u8 {
        self.consecutive_compaction_failures
    }

    /// Last input-token count seen on a provider response.
    pub fn last_input_tokens(&self) -> u64 {
        self.last_input_tokens
    }

    /// Last output-token count seen on a provider response.
    pub fn last_output_tokens(&self) -> u64 {
        self.last_output_tokens
    }

    /// The currently-known model context window. `0` means unknown —
    /// the guard runs as a no-op in that case.
    pub fn context_window(&self) -> u64 {
        self.context_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_context_window_always_ok() {
        let guard = ContextGuard::new();
        assert_eq!(guard.check(), ContextCheckResult::Ok);
    }

    #[test]
    fn low_utilization_is_ok() {
        let mut guard = ContextGuard::with_context_window(100_000);
        guard.update_usage(&UsageInfo {
            input_tokens: 10_000,
            output_tokens: 5_000,
            context_window: 100_000,
            ..Default::default()
        });
        assert_eq!(guard.check(), ContextCheckResult::Ok);
    }

    #[test]
    fn high_utilization_triggers_compaction() {
        let mut guard = ContextGuard::with_context_window(100_000);
        guard.update_usage(&UsageInfo {
            input_tokens: 85_000,
            output_tokens: 6_000,
            context_window: 100_000,
            ..Default::default()
        });
        assert_eq!(guard.check(), ContextCheckResult::CompactionNeeded);
    }

    #[test]
    fn circuit_breaker_trips_after_three_failures() {
        let mut guard = ContextGuard::with_context_window(100_000);
        guard.update_usage(&UsageInfo {
            input_tokens: 90_000,
            output_tokens: 6_000,
            context_window: 100_000,
            ..Default::default()
        });

        guard.record_compaction_failure();
        guard.record_compaction_failure();
        assert!(!guard.is_compaction_disabled());

        guard.record_compaction_failure();
        assert!(guard.is_compaction_disabled());

        // Now at >95%, should return exhausted
        assert!(matches!(
            guard.check(),
            ContextCheckResult::ContextExhausted { .. }
        ));
    }

    #[test]
    fn success_resets_circuit_breaker() {
        let mut guard = ContextGuard::with_context_window(100_000);
        guard.record_compaction_failure();
        guard.record_compaction_failure();
        guard.record_compaction_failure();
        assert!(guard.is_compaction_disabled());

        guard.record_compaction_success();
        assert!(!guard.is_compaction_disabled());
        assert_eq!(guard.consecutive_failures(), 0);
    }
}
