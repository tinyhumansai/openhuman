//! Command-center control verbs (issue #3373).
//!
//! The read-only projection in [`super::ops`] shows what background agent work
//! is in flight; these verbs let a reviewer *act* on a single row. Each verb is
//! a durable transition on the run ledger (`tinyagents_session::run_ledger`):
//!
//! - **stop** — cancel a non-terminal run (→ `cancelled`).
//! - **retry** — re-queue a finished-with-error run (`failed` / `cancelled` /
//!   `interrupted` → `pending`), clearing the stale error + completion time.
//! - **continue** — answer an `awaiting_user` run so it can resume (→ `running`).
//! - **follow_up** — record a follow-up instruction against any run, leaving its
//!   status unchanged (mirrors recording a parent→child message).
//!
//! These mirror the in-memory [`AgentOrchestrationSession`] control plane
//! (`close_agent` / `resume_agent` / `follow_up` / `message_agent`) but operate
//! on the *durable* ledger, so they survive restart and apply to any tracked
//! run rather than only the children of a live session. They persist the new
//! status (via [`transition_agent_run_status`], which can clear `error` /
//! `completed_at` — the upsert path cannot) and append a `run_event` recording
//! the action for the run's timeline.
//!
//! The allowed-transition matrix lives in the pure [`plan_transition`], which is
//! unit-tested without a database, mirroring [`super::ops::build_view`].
//!
//! [`AgentOrchestrationSession`]: crate::openhuman::agent::orchestration::ops::AgentOrchestrationSession
//! [`transition_agent_run_status`]: tinyagents_session::run_ledger::transition_agent_run_status

use chrono::{DateTime, Utc};
use serde_json::json;
use thiserror::Error;

use crate::openhuman::config::Config;
use tinyagents_session::run_ledger::{
    append_run_event, get_agent_run, transition_agent_run_status, AgentRunStatus, RunEventAppend,
};

use super::ops::project_row;
use super::types::AgentWorkRow;

/// A control action a reviewer can take on a command-center row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlVerb {
    /// Cancel an in-flight run.
    Stop,
    /// Re-queue a run that finished with an error.
    Retry,
    /// Answer an `awaiting_user` run so it can resume.
    Continue,
    /// Record a follow-up instruction against a run.
    FollowUp,
}

impl ControlVerb {
    /// Parse the wire `action` string. Returns `None` for an unknown verb.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "stop" => Some(Self::Stop),
            "retry" => Some(Self::Retry),
            "continue" => Some(Self::Continue),
            "follow_up" => Some(Self::FollowUp),
            _ => None,
        }
    }

    /// Stable wire string for this verb.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Retry => "retry",
            Self::Continue => "continue",
            Self::FollowUp => "follow_up",
        }
    }

    /// Whether this verb requires a non-empty user `message`.
    ///
    /// `continue` carries the answer that unblocks an `awaiting_user` run, and
    /// `follow_up` carries the new instruction — both are meaningless empty.
    pub fn requires_message(self) -> bool {
        matches!(self, Self::Continue | Self::FollowUp)
    }
}

/// Why a control verb could not be applied.
#[derive(Debug, Error)]
pub enum ControlError {
    /// No run matched the supplied id.
    #[error("agent run '{0}' not found")]
    RunNotFound(String),
    /// The verb is not legal from the run's current status.
    #[error("'{verb}' is not allowed while run is '{status}'")]
    InvalidTransition {
        verb: &'static str,
        status: &'static str,
    },
    /// The verb requires a message and none was supplied.
    #[error("'{0}' requires a non-empty message")]
    MessageRequired(&'static str),
    /// A durable run-ledger read/write failed.
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// Lets `?` carry a run-ledger failure straight into [`ControlError`].
///
/// The ledger lives in `tinyagents` and speaks `TinyAgentsError`, while this
/// module's callers speak `anyhow`. Without this the `#[from] anyhow::Error`
/// variant does not apply, because `TinyAgentsError` is a distinct type — so
/// every ledger call in this file would need its own `map_err`.
impl From<tinyagents_harness::TinyAgentsError> for ControlError {
    fn from(err: tinyagents_harness::TinyAgentsError) -> Self {
        Self::Storage(err.into())
    }
}

/// The durable status a verb moves a run to, plus the event type to record.
///
/// `error` / `completed_at` handling is verb-specific and applied in
/// [`apply_control`]; only the status move + event name are decided here so the
/// transition legality stays purely testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControlPlan {
    target_status: AgentRunStatus,
    event_type: &'static str,
}

/// Decide whether `verb` is legal from `current` and, if so, where it lands.
///
/// Pure: no I/O. The matrix is exhaustive on the verb so a new verb fails to
/// compile until its transition rule is decided.
fn plan_transition(
    current: AgentRunStatus,
    verb: ControlVerb,
) -> Result<ControlPlan, ControlError> {
    let invalid = || ControlError::InvalidTransition {
        verb: verb.as_str(),
        status: current.as_str(),
    };
    match verb {
        // Stop only makes sense while the run is still live.
        ControlVerb::Stop => {
            if current.is_terminal() {
                Err(invalid())
            } else {
                Ok(ControlPlan {
                    target_status: AgentRunStatus::Cancelled,
                    event_type: "control_stopped",
                })
            }
        }
        // Retry re-queues a run that finished with an error (failed) or was
        // stopped (cancelled / interrupted). A successfully completed run has
        // nothing to retry; a live run is not yet retryable.
        ControlVerb::Retry => match current {
            AgentRunStatus::Failed | AgentRunStatus::Cancelled | AgentRunStatus::Interrupted => {
                Ok(ControlPlan {
                    target_status: AgentRunStatus::Pending,
                    event_type: "control_retry",
                })
            }
            _ => Err(invalid()),
        },
        // Continue answers a run that is explicitly blocked on the user.
        ControlVerb::Continue => {
            if current == AgentRunStatus::AwaitingUser {
                Ok(ControlPlan {
                    target_status: AgentRunStatus::Running,
                    event_type: "control_continued",
                })
            } else {
                Err(invalid())
            }
        }
        // Follow-up just records a new instruction; the run keeps its status
        // (you can follow up on a completed run as easily as a running one).
        ControlVerb::FollowUp => Ok(ControlPlan {
            target_status: current,
            event_type: "control_follow_up",
        }),
    }
}

/// Apply a control verb to one background agent run.
///
/// Validates the message requirement, loads the run, checks the transition is
/// legal for its current status, persists the new status (clearing or setting
/// `error` / `completed_at` per verb), and appends a `run_event` capturing the
/// action. Returns the freshly re-projected [`AgentWorkRow`].
///
/// Errors: [`ControlError::MessageRequired`] when a message-bearing verb has no
/// message, [`ControlError::RunNotFound`] for an unknown run id,
/// [`ControlError::InvalidTransition`] for an illegal move, or
/// [`ControlError::Storage`] for a ledger failure.
pub fn apply_control(
    config: &Config,
    run_id: &str,
    verb: ControlVerb,
    message: Option<&str>,
    reason: Option<&str>,
) -> Result<AgentWorkRow, ControlError> {
    let message = message.map(str::trim).filter(|s| !s.is_empty());
    let reason = reason.map(str::trim).filter(|s| !s.is_empty());
    log::debug!(
        target: "command_center",
        "[command_center] apply_control.entry run_id={run_id} verb={} has_message={} has_reason={}",
        verb.as_str(),
        message.is_some(),
        reason.is_some()
    );

    if verb.requires_message() && message.is_none() {
        log::debug!(
            target: "command_center",
            "[command_center] apply_control.message_required run_id={run_id} verb={}",
            verb.as_str()
        );
        return Err(ControlError::MessageRequired(verb.as_str()));
    }

    let run = get_agent_run(&config.workspace_dir, run_id)?
        .ok_or_else(|| ControlError::RunNotFound(run_id.to_string()))?;
    let from_status = run.status;
    let plan = plan_transition(from_status, verb)?;

    // Verb-specific error / completion handling. The transition op writes both
    // columns verbatim, so `None` clears them.
    let (next_error, next_completed_at): (Option<String>, Option<DateTime<Utc>>) = match verb {
        // Stopping records the optional reason and stamps completion now.
        ControlVerb::Stop => (reason.map(str::to_string), Some(Utc::now())),
        // Re-queuing drops the stale failure reason and completion time.
        ControlVerb::Retry | ControlVerb::Continue => (None, None),
        // Follow-up leaves the run as-is.
        ControlVerb::FollowUp => (run.error.clone(), run.completed_at),
    };

    let updated = transition_agent_run_status(
        &config.workspace_dir,
        run_id,
        plan.target_status,
        next_error.as_deref(),
        next_completed_at,
    )?
    .ok_or_else(|| ControlError::RunNotFound(run_id.to_string()))?;

    // Record the action on the run's durable timeline.
    append_run_event(
        &config.workspace_dir,
        RunEventAppend {
            run_id: run_id.to_string(),
            event_type: plan.event_type.to_string(),
            payload: json!({
                "verb": verb.as_str(),
                "fromStatus": from_status.as_str(),
                "toStatus": plan.target_status.as_str(),
                "message": message,
                "reason": reason,
            }),
        },
    )?;

    log::debug!(
        target: "command_center",
        "[command_center] apply_control.done run_id={run_id} verb={} from={} to={}",
        verb.as_str(),
        from_status.as_str(),
        plan.target_status.as_str()
    );
    Ok(project_row(updated))
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
