//! Domain types for the thread-level goal.
//!
//! A **thread goal** is a single, thread-scoped "completion contract" — a
//! durable objective the agent keeps pursuing across turns, interrupts,
//! resumes, and budget boundaries. It is distinct from the global
//! [`memory_goals`](crate::openhuman::memory_goals) list (long-term, workspace
//! wide) and from the per-thread kanban task board: there is **exactly one**
//! goal per thread, with a small lifecycle and optional token budget.
//!
//! The shape mirrors OpenAI Codex's `thread_goals` row, adapted to OpenHuman's
//! per-thread file-JSON persistence (see [`super::store`]).

use serde::{Deserialize, Serialize};

/// Lifecycle state of a thread goal.
///
/// Ownership is **asymmetric** (Codex parity): the model may create/replace a
/// goal and mark it `Complete`; `Paused` / `BudgetLimited` are system-driven
/// (interrupt/abort and accounting respectively), and clearing deletes the row
/// entirely rather than being a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadGoalStatus {
    /// The agent may make progress and (when idle) auto-continue.
    Active,
    /// Work is suspended (user interrupt/abort); the objective persists and is
    /// reactivated on thread resume.
    Paused,
    /// The token budget has been reached; substantive work halts until the user
    /// raises the budget or clears the goal.
    BudgetLimited,
    /// Evidence confirms the objective is satisfied.
    Complete,
}

impl ThreadGoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::BudgetLimited => "budget_limited",
            Self::Complete => "complete",
        }
    }

    /// Whether the goal is in a state where the agent should keep working it
    /// (and idle auto-continuation may fire).
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Whether the goal is in a terminal state for continuation purposes —
    /// `Complete` or `BudgetLimited` never auto-continue.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::BudgetLimited)
    }
}

/// A single thread-scoped goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoal {
    /// The thread this goal belongs to (one goal per thread).
    pub thread_id: String,
    /// Version identifier, re-minted on **every objective replacement**. Stale
    /// accounting writes that pass a non-matching `expected_goal_id` are
    /// silently ignored — see [`super::store::account_usage`].
    pub goal_id: String,
    /// The durable objective, one or more sentences.
    pub objective: String,
    /// Lifecycle state.
    pub status: ThreadGoalStatus,
    /// Optional token ceiling. When set and `tokens_used >= token_budget`, the
    /// goal transitions to [`ThreadGoalStatus::BudgetLimited`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    /// Cumulative tokens accounted against this goal.
    #[serde(default)]
    pub tokens_used: u64,
    /// Cumulative wall-clock seconds accounted against this goal.
    #[serde(default)]
    pub time_used_seconds: u64,
    /// Creation time (unix epoch milliseconds).
    pub created_at_ms: u64,
    /// Last-mutation time (unix epoch milliseconds).
    pub updated_at_ms: u64,
    /// Set when an idle auto-continuation turn produced **zero tool calls**, to
    /// stop a continuation loop. Cleared on any user action, tool execution, or
    /// external mutation (e.g. `goal_set`).
    #[serde(default)]
    pub continuation_suppressed: bool,
}

impl ThreadGoal {
    /// Tokens remaining before the budget cap, if a budget is set.
    pub fn budget_remaining(&self) -> Option<u64> {
        self.token_budget
            .map(|b| b.saturating_sub(self.tokens_used))
    }

    /// Whether accounting has reached or exceeded the configured budget.
    pub fn over_budget(&self) -> bool {
        matches!(self.token_budget, Some(b) if self.tokens_used >= b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strings_match_serialized() {
        assert_eq!(ThreadGoalStatus::Active.as_str(), "active");
        assert_eq!(ThreadGoalStatus::Paused.as_str(), "paused");
        assert_eq!(ThreadGoalStatus::BudgetLimited.as_str(), "budget_limited");
        assert_eq!(ThreadGoalStatus::Complete.as_str(), "complete");
    }

    #[test]
    fn active_and_terminal_predicates() {
        assert!(ThreadGoalStatus::Active.is_active());
        assert!(!ThreadGoalStatus::Paused.is_active());
        assert!(ThreadGoalStatus::Complete.is_terminal());
        assert!(ThreadGoalStatus::BudgetLimited.is_terminal());
        assert!(!ThreadGoalStatus::Active.is_terminal());
    }

    #[test]
    fn budget_helpers() {
        let mut g = ThreadGoal {
            thread_id: "t".into(),
            goal_id: "g".into(),
            objective: "do it".into(),
            status: ThreadGoalStatus::Active,
            token_budget: Some(100),
            tokens_used: 40,
            time_used_seconds: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
            continuation_suppressed: false,
        };
        assert_eq!(g.budget_remaining(), Some(60));
        assert!(!g.over_budget());
        g.tokens_used = 120;
        assert_eq!(g.budget_remaining(), Some(0));
        assert!(g.over_budget());
        g.token_budget = None;
        assert_eq!(g.budget_remaining(), None);
        assert!(!g.over_budget());
    }
}
