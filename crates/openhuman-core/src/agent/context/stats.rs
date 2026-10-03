//! Context utilisation for [`ContextManager`].
//!
//! Live context reduction is owned by the TinyAgents middleware stack. This
//! module keeps only OpenHuman-specific bookkeeping: last provider usage,
//! and context-window utilisation for the UI/footer.

use crate::inference::provider::BilledUsage;

#[derive(Debug, Default)]
pub(crate) struct ContextStatsState {
    last_input_tokens: u64,
    last_output_tokens: u64,
    context_window: u64,
}

impl ContextStatsState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record_usage(&mut self, usage: &BilledUsage) {
        self.last_input_tokens = usage.input_tokens;
        self.last_output_tokens = usage.output_tokens;
        if usage.context_window() > 0 {
            self.context_window = usage.context_window();
        }
    }

    pub(crate) fn utilization_pct(&self) -> Option<u8> {
        if self.context_window == 0 {
            return None;
        }
        let total_used = self.last_input_tokens + self.last_output_tokens;
        let pct = (total_used as f64 / self.context_window as f64 * 100.0).round();
        Some(pct as u8)
    }

    pub(crate) fn last_input_tokens(&self) -> u64 {
        self.last_input_tokens
    }

    pub(crate) fn last_output_tokens(&self) -> u64 {
        self.last_output_tokens
    }

    pub(crate) fn context_window(&self) -> u64 {
        self.context_window
    }
}
