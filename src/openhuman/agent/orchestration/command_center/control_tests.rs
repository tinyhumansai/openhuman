use super::*;
use serde_json::json;
use tempfile::TempDir;
use tinyagents_session::run_ledger::{
    list_recent_run_events, upsert_agent_run, AgentRunKind, AgentRunUpsert, RunEventListRequest,
};

fn test_config(dir: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.action_dir = dir.path().join("actions");
    config
}

fn seed_run(config: &Config, id: &str, status: AgentRunStatus) {
    upsert_agent_run(
        &config.workspace_dir,
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
        &config.workspace_dir,
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
    let run = get_agent_run(&config.workspace_dir, "run-1")
        .unwrap()
        .unwrap();
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
        &config.workspace_dir,
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
