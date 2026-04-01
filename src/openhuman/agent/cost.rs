//! Token cost tracking for agent loop budget enforcement.
//!
//! Tracks cumulative token usage across inference calls and enforces
//! the `max_cost_per_day_cents` budget from the security policy.

use crate::openhuman::providers::UsageInfo;
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-token pricing in microdollars (millionths of a dollar).
/// Default pricing is conservative; callers should provide model-specific rates.
#[derive(Debug, Clone)]
pub struct TokenPricing {
    /// Cost per input token in microdollars.
    pub input_token_microdollars: u64,
    /// Cost per output token in microdollars.
    pub output_token_microdollars: u64,
}

impl Default for TokenPricing {
    fn default() -> Self {
        // Conservative defaults (~$3/1M input, ~$15/1M output — Sonnet-class)
        Self {
            input_token_microdollars: 3,
            output_token_microdollars: 15,
        }
    }
}

/// Thread-safe cumulative token and cost tracker.
#[derive(Debug)]
pub struct CostTracker {
    total_input_tokens: AtomicU64,
    total_output_tokens: AtomicU64,
    total_cost_microdollars: AtomicU64,
    pricing: TokenPricing,
    /// Budget in microdollars (0 = unlimited).
    budget_microdollars: u64,
}

impl CostTracker {
    /// Create a new tracker with the given pricing and budget (in cents).
    pub fn new(pricing: TokenPricing, budget_cents: u32) -> Self {
        Self {
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
            total_cost_microdollars: AtomicU64::new(0),
            pricing,
            budget_microdollars: budget_cents as u64 * 10_000, // cents → microdollars
        }
    }

    /// Create a tracker with default pricing and a budget in cents.
    pub fn with_budget_cents(budget_cents: u32) -> Self {
        Self::new(TokenPricing::default(), budget_cents)
    }

    /// Record usage from a provider response.
    pub fn record_usage(&self, usage: &UsageInfo) {
        self.total_input_tokens
            .fetch_add(usage.input_tokens, Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(usage.output_tokens, Ordering::Relaxed);

        let cost = usage.input_tokens * self.pricing.input_token_microdollars
            + usage.output_tokens * self.pricing.output_token_microdollars;
        self.total_cost_microdollars
            .fetch_add(cost, Ordering::Relaxed);

        tracing::debug!(
            input_tokens = usage.input_tokens,
            output_tokens = usage.output_tokens,
            cost_microdollars = cost,
            total_cost_microdollars = self.total_cost_microdollars.load(Ordering::Relaxed),
            "[cost_tracker] recorded usage"
        );
    }

    /// Check whether the budget has been exceeded.
    /// Returns `Ok(())` if within budget, or `Err` with spent/budget amounts.
    pub fn check_budget(&self) -> Result<(), (u64, u64)> {
        if self.budget_microdollars == 0 {
            return Ok(()); // Unlimited
        }
        let spent = self.total_cost_microdollars.load(Ordering::Relaxed);
        if spent > self.budget_microdollars {
            Err((spent, self.budget_microdollars))
        } else {
            Ok(())
        }
    }

    /// Get the total input tokens recorded.
    pub fn total_input_tokens(&self) -> u64 {
        self.total_input_tokens.load(Ordering::Relaxed)
    }

    /// Get the total output tokens recorded.
    pub fn total_output_tokens(&self) -> u64 {
        self.total_output_tokens.load(Ordering::Relaxed)
    }

    /// Get the total cost in microdollars.
    pub fn total_cost_microdollars(&self) -> u64 {
        self.total_cost_microdollars.load(Ordering::Relaxed)
    }

    /// Human-readable cost summary.
    pub fn summary(&self) -> String {
        let input = self.total_input_tokens.load(Ordering::Relaxed);
        let output = self.total_output_tokens.load(Ordering::Relaxed);
        let cost = self.total_cost_microdollars.load(Ordering::Relaxed);
        let dollars = cost as f64 / 1_000_000.0;
        format!("Tokens: {input} in / {output} out | Cost: ${dollars:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_cumulative_usage() {
        let tracker = CostTracker::with_budget_cents(1000);
        tracker.record_usage(&UsageInfo {
            input_tokens: 1000,
            output_tokens: 500,
            context_window: 0,
        });
        tracker.record_usage(&UsageInfo {
            input_tokens: 2000,
            output_tokens: 1000,
            context_window: 0,
        });

        assert_eq!(tracker.total_input_tokens(), 3000);
        assert_eq!(tracker.total_output_tokens(), 1500);
        assert!(tracker.total_cost_microdollars() > 0);
    }

    #[test]
    fn budget_enforcement() {
        // Budget: 1 cent = 10,000 microdollars
        let tracker = CostTracker::new(
            TokenPricing {
                input_token_microdollars: 100,
                output_token_microdollars: 100,
            },
            1, // 1 cent
        );

        // 50 input + 50 output = 100 tokens × 100 = 10,000 microdollars = 1 cent (at limit)
        tracker.record_usage(&UsageInfo {
            input_tokens: 50,
            output_tokens: 50,
            context_window: 0,
        });
        assert!(tracker.check_budget().is_ok());

        // One more token pushes over budget
        tracker.record_usage(&UsageInfo {
            input_tokens: 1,
            output_tokens: 0,
            context_window: 0,
        });
        assert!(tracker.check_budget().is_err());
    }

    #[test]
    fn unlimited_budget() {
        let tracker = CostTracker::with_budget_cents(0);
        tracker.record_usage(&UsageInfo {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            context_window: 0,
        });
        assert!(tracker.check_budget().is_ok());
    }

    #[test]
    fn summary_format() {
        let tracker = CostTracker::with_budget_cents(100);
        tracker.record_usage(&UsageInfo {
            input_tokens: 1000,
            output_tokens: 500,
            context_window: 0,
        });
        let summary = tracker.summary();
        assert!(summary.contains("1000 in"));
        assert!(summary.contains("500 out"));
        assert!(summary.contains("$"));
    }
}
