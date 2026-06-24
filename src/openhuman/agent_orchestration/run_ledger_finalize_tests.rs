//! Tests for the global-bus run-ledger finalizer.

use super::*;

use serde_json::json;
use tempfile::TempDir;

use crate::core::event_bus::EventHandler;
use crate::openhuman::session_db::run_ledger::{
    get_agent_run, upsert_agent_run, AgentRunKind, AgentRunStatus, AgentRunUpsert,
};

fn test_config(dir: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.action_dir = dir.path().join("actions");
    config
}

fn seed_running(config: &Config, id: &str) {
    upsert_agent_run(
        config,
        AgentRunUpsert {
            id: id.into(),
            kind: AgentRunKind::Subagent,
            parent_run_id: Some("parent-turn".into()),
            parent_thread_id: Some("thread-1".into()),
            agent_id: Some("tinyplace_agent".into()),
            status: AgentRunStatus::Running,
            prompt_ref: None,
            worker_thread_id: None,
            task_board_id: None,
            task_card_id: None,
            checkpoint_path: None,
            checkpoint: None,
            summary: None,
            error: None,
            metadata: json!({}),
            started_at: None,
            completed_at: None,
        },
    )
    .unwrap();
}

#[tokio::test]
async fn settles_running_run_on_subagent_completed() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    seed_running(&config, "sub-1");

    let sub = RunLedgerFinalizeSubscriber {
        config: config.clone(),
    };
    sub.handle(&DomainEvent::SubagentCompleted {
        parent_session: "session-1".into(),
        task_id: "sub-1".into(),
        agent_id: "tinyplace_agent".into(),
        elapsed_ms: 1234,
        output_chars: 760,
        iterations: 3,
    })
    .await;

    let run = get_agent_run(&config, "sub-1")
        .unwrap()
        .expect("run present");
    assert_eq!(run.status, AgentRunStatus::Completed);
    assert!(run.completed_at.is_some());
}

#[tokio::test]
async fn settles_running_run_on_subagent_failed_with_error() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    seed_running(&config, "sub-2");

    let sub = RunLedgerFinalizeSubscriber {
        config: config.clone(),
    };
    sub.handle(&DomainEvent::SubagentFailed {
        parent_session: "session-1".into(),
        task_id: "sub-2".into(),
        agent_id: "tinyplace_agent".into(),
        error: "boom".into(),
    })
    .await;

    let run = get_agent_run(&config, "sub-2")
        .unwrap()
        .expect("run present");
    assert_eq!(run.status, AgentRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn ignores_unrelated_events_and_missing_runs() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let sub = RunLedgerFinalizeSubscriber {
        config: config.clone(),
    };
    // Completion for a run that was never recorded — no-op, no panic.
    sub.handle(&DomainEvent::SubagentCompleted {
        parent_session: "session-1".into(),
        task_id: "ghost".into(),
        agent_id: "tinyplace_agent".into(),
        elapsed_ms: 1,
        output_chars: 0,
        iterations: 1,
    })
    .await;
    assert!(get_agent_run(&config, "ghost").unwrap().is_none());
}
