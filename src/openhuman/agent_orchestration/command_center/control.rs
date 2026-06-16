//! Command-center control verbs (issue #3373).
//!
//! The read-only projection in [`super::ops`] shows what background agent work
//! is in flight; these verbs let a reviewer *act* on a single row. Each verb is
//! a durable transition on the run ledger (`session_db::run_ledger`):
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
//! [`AgentOrchestrationSession`]: crate::openhuman::agent_orchestration::ops::AgentOrchestrationSession
//! [`transition_agent_run_status`]: crate::openhuman::session_db::run_ledger::transition_agent_run_status

use chrono::{DateTime, Utc};
use serde_json::json;
use thiserror::Error;

use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::{
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

    let run = get_agent_run(config, run_id)?
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
        config,
        run_id,
        plan.target_status,
        next_error.as_deref(),
        next_completed_at,
    )?
    .ok_or_else(|| ControlError::RunNotFound(run_id.to_string()))?;

    // Record the action on the run's durable timeline.
    append_run_event(
        config,
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
mod tests {
    use super::*;
    use crate::openhuman::session_db::run_ledger::{
        list_recent_run_events, upsert_agent_run, AgentRunKind, AgentRunUpsert, RunEventListRequest,
    };
    use serde_json::json;
    use tempfile::TempDir;

    fn test_config(dir: &TempDir) -> Config {
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config.action_dir = dir.path().join("actions");
        config
    }

    fn seed_run(config: &Config, id: &str, status: AgentRunStatus) {
        upsert_agent_run(
            config,
            AgentRunUpsert {
                id: id.to_string(),
                kind: AgentRunKind::Subagent,
                parent_run_id: None,
                parent_thread_id: Some("thread-1".into()),
                agent_id: Some("researcher".into()),
                status,
                prompt_ref: None,
                worker_thread_id: None,
                task_board_id: None,
                task_card_id: None,
                checkpoint_path: None,
                checkpoint: None,
                summary: None,
                error: if status == AgentRunStatus::Failed {
                    Some("boom".into())
                } else {
                    None
                },
                metadata: json!({}),
                started_at: None,
                completed_at: if status.is_terminal() {
                    Some(Utc::now())
                } else {
                    None
                },
            },
        )
        .unwrap();
    }

    // ---- pure planner -----------------------------------------------------

    #[test]
    fn parse_round_trips_known_verbs_and_rejects_unknown() {
        for verb in [
            ControlVerb::Stop,
            ControlVerb::Retry,
            ControlVerb::Continue,
            ControlVerb::FollowUp,
        ] {
            assert_eq!(ControlVerb::parse(verb.as_str()), Some(verb));
        }
        assert_eq!(ControlVerb::parse("nonsense"), None);
        assert_eq!(ControlVerb::parse(" stop "), Some(ControlVerb::Stop));
    }

    #[test]
    fn message_requirement_is_verb_specific() {
        assert!(ControlVerb::Continue.requires_message());
        assert!(ControlVerb::FollowUp.requires_message());
        assert!(!ControlVerb::Stop.requires_message());
        assert!(!ControlVerb::Retry.requires_message());
    }

    #[test]
    fn stop_allowed_only_while_non_terminal() {
        for status in [
            AgentRunStatus::Pending,
            AgentRunStatus::Running,
            AgentRunStatus::AwaitingUser,
            AgentRunStatus::Paused,
        ] {
            let plan = plan_transition(status, ControlVerb::Stop).unwrap();
            assert_eq!(plan.target_status, AgentRunStatus::Cancelled);
            assert_eq!(plan.event_type, "control_stopped");
        }
        for status in [
            AgentRunStatus::Completed,
            AgentRunStatus::Failed,
            AgentRunStatus::Cancelled,
            AgentRunStatus::Interrupted,
        ] {
            assert!(matches!(
                plan_transition(status, ControlVerb::Stop),
                Err(ControlError::InvalidTransition { .. })
            ));
        }
    }

    #[test]
    fn retry_allowed_only_from_error_terminals() {
        for status in [
            AgentRunStatus::Failed,
            AgentRunStatus::Cancelled,
            AgentRunStatus::Interrupted,
        ] {
            let plan = plan_transition(status, ControlVerb::Retry).unwrap();
            assert_eq!(plan.target_status, AgentRunStatus::Pending);
            assert_eq!(plan.event_type, "control_retry");
        }
        for status in [
            AgentRunStatus::Pending,
            AgentRunStatus::Running,
            AgentRunStatus::AwaitingUser,
            AgentRunStatus::Paused,
            AgentRunStatus::Completed,
        ] {
            assert!(matches!(
                plan_transition(status, ControlVerb::Retry),
                Err(ControlError::InvalidTransition { .. })
            ));
        }
    }

    #[test]
    fn continue_allowed_only_from_awaiting_user() {
        let plan = plan_transition(AgentRunStatus::AwaitingUser, ControlVerb::Continue).unwrap();
        assert_eq!(plan.target_status, AgentRunStatus::Running);
        assert_eq!(plan.event_type, "control_continued");
        for status in [
            AgentRunStatus::Pending,
            AgentRunStatus::Running,
            AgentRunStatus::Paused,
            AgentRunStatus::Completed,
            AgentRunStatus::Failed,
        ] {
            assert!(matches!(
                plan_transition(status, ControlVerb::Continue),
                Err(ControlError::InvalidTransition { .. })
            ));
        }
    }

    #[test]
    fn follow_up_keeps_status_from_any_state() {
        for status in [
            AgentRunStatus::Running,
            AgentRunStatus::Completed,
            AgentRunStatus::Failed,
        ] {
            let plan = plan_transition(status, ControlVerb::FollowUp).unwrap();
            assert_eq!(plan.target_status, status);
            assert_eq!(plan.event_type, "control_follow_up");
        }
    }

    // ---- ledger-backed apply ---------------------------------------------

    #[test]
    fn stop_cancels_a_running_run_and_records_an_event() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        seed_run(&config, "run-1", AgentRunStatus::Running);

        let row = apply_control(&config, "run-1", ControlVerb::Stop, None, Some("manual")).unwrap();
        assert_eq!(row.status, "cancelled");
        assert_eq!(row.bucket.as_str(), "stopped");
        assert_eq!(row.error.as_deref(), Some("manual"));

        let events = list_recent_run_events(
            &config,
            &RunEventListRequest {
                run_id: "run-1".into(),
                after_sequence: None,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].event_type, "control_stopped");
        assert_eq!(events.events[0].payload["toStatus"], "cancelled");
    }

    #[test]
    fn retry_requeues_a_failed_run_and_clears_error() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        seed_run(&config, "run-1", AgentRunStatus::Failed);

        let row = apply_control(&config, "run-1", ControlVerb::Retry, None, None).unwrap();
        assert_eq!(row.status, "pending");
        assert_eq!(row.bucket.as_str(), "working");
        // The stale failure reason is dropped (upsert COALESCE could not do this).
        assert_eq!(row.error, None);
    }

    #[test]
    fn continue_resumes_an_awaiting_user_run() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        seed_run(&config, "run-1", AgentRunStatus::AwaitingUser);

        let row = apply_control(
            &config,
            "run-1",
            ControlVerb::Continue,
            Some("use the staging bucket"),
            None,
        )
        .unwrap();
        assert_eq!(row.status, "running");
        assert_eq!(row.bucket.as_str(), "working");
    }

    #[test]
    fn continue_without_message_is_rejected_before_touching_the_ledger() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        seed_run(&config, "run-1", AgentRunStatus::AwaitingUser);

        let err =
            apply_control(&config, "run-1", ControlVerb::Continue, Some("   "), None).unwrap_err();
        assert!(matches!(err, ControlError::MessageRequired("continue")));
        // Status untouched.
        let run = get_agent_run(&config, "run-1").unwrap().unwrap();
        assert_eq!(run.status, AgentRunStatus::AwaitingUser);
    }

    #[test]
    fn follow_up_records_an_event_without_changing_status() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        seed_run(&config, "run-1", AgentRunStatus::Completed);

        let row = apply_control(
            &config,
            "run-1",
            ControlVerb::FollowUp,
            Some("now summarize it"),
            None,
        )
        .unwrap();
        assert_eq!(row.status, "completed");

        let events = list_recent_run_events(
            &config,
            &RunEventListRequest {
                run_id: "run-1".into(),
                after_sequence: None,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(events.events[0].event_type, "control_follow_up");
        assert_eq!(events.events[0].payload["message"], "now summarize it");
    }

    #[test]
    fn invalid_transition_is_rejected() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        seed_run(&config, "run-1", AgentRunStatus::Completed);

        let err = apply_control(&config, "run-1", ControlVerb::Stop, None, None).unwrap_err();
        assert!(matches!(
            err,
            ControlError::InvalidTransition {
                verb: "stop",
                status: "completed"
            }
        ));
    }

    #[test]
    fn unknown_run_is_not_found() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let err = apply_control(&config, "ghost", ControlVerb::Stop, None, None).unwrap_err();
        assert!(matches!(err, ControlError::RunNotFound(id) if id == "ghost"));
    }
}
