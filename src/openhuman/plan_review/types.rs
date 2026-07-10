//! Types for the interactive plan-review gate.
//!
//! A plan review parks a live (interactive) agent turn after the orchestrator
//! has laid out a thread-scoped plan, surfaces the plan to the user, and
//! resumes the SAME turn with the user's decision. Unlike the task-board
//! approval lifecycle (which the background dispatcher runs on the
//! `user-tasks` / `task-sources` boards), this gate is a parked-future on the
//! live turn — modelled on [`crate::openhuman::approval::ApprovalGate`] but
//! in-memory only: an interactive turn that can't resume across a restart has
//! nothing to persist.

use serde::{Deserialize, Serialize};

/// The user's decision on a parked plan, sent back to the parked turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PlanReviewResolution {
    /// Run the plan as proposed — the parked turn resumes and executes.
    Approve,
    /// Drop the plan — the parked turn resumes and stops without executing.
    Reject,
    /// Revise the plan per `feedback` — the parked turn resumes, re-plans, and
    /// re-parks for another review.
    Revise { feedback: String },
}

impl PlanReviewResolution {
    /// Stable label for events/logs (the `Revise` payload is omitted here —
    /// feedback is user content and must not land in redacted event fields).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
            Self::Revise { .. } => "revise",
        }
    }
}
