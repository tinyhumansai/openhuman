//! Read-out of the latest completed turn: usage totals and the iteration-cap
//! flag.

use super::super::types::OpenHumanSessionHost;

impl OpenHumanSessionHost {
    /// Borrow the holistic token/cost/context totals for the latest completed
    /// turn (parent + sub-agents) **without consuming them**. `None` until a
    /// turn has run.
    ///
    /// This is the public, non-draining counterpart to
    /// [`take_last_turn_usage_totals`](Self::take_last_turn_usage_totals): a
    /// downstream crate embedding OpenHuman as a library (e.g. the OpenCompany
    /// hosting platform's cost-metering hook) can read per-turn token and USD
    /// totals after [`OpenHumanSessionHost::turn`](crate::agent::OpenHumanSessionHost) returns,
    /// while leaving the value in place for the web-channel drain path.
    pub fn last_turn_usage(&self) -> Option<crate::agent::tinyagents::host::LastTurnUsage> {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_turn_usage
            .clone()
    }

    /// Drain and return the holistic token/cost/context totals for the latest
    /// completed turn (parent + sub-agents). `None` until a turn has run.
    /// Consumed by web-channel delivery to populate the `chat_done` usage fields.
    pub(crate) fn take_last_turn_usage_totals(
        &mut self,
    ) -> Option<crate::agent::tinyagents::host::LastTurnUsage> {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_turn_usage
            .take()
    }

    /// Whether the most recently completed [`Self::turn`] / [`Self::run_single`]
    /// paused because it hit `max_tool_iterations`, rather than finishing
    /// naturally (see the field doc on `last_turn_hit_cap`). `false` before
    /// any turn has run. Not draining — unlike the usage totals above, a
    /// caller may reasonably check this more than once per turn.
    pub fn last_turn_hit_cap(&self) -> bool {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_turn_hit_cap
    }
}
