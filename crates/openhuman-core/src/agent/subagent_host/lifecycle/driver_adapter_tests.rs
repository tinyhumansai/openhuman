use super::*;
use tinyagents_orchestration::subagent::SubagentPersistenceDisposition;

fn key() -> SubagentTaskKey {
    SubagentTaskKey {
        root_run_id: "root".into(),
        parent_run_id: "parent".into(),
        thread_id: Some("thread".into()),
        task_id: "task".into(),
    }
}

fn cause(
    class: &str,
    operation: &str,
    summary: &str,
) -> tinyagents_harness::no_progress::ClassifiedStopCause {
    tinyagents_harness::no_progress::ClassifiedStopCause {
        class: class.into(),
        operation: operation.into(),
        summary: summary.into(),
    }
}

fn neutral_outcome(
    status: SubagentOutcomeKind,
    cause: &tinyagents_harness::no_progress::ClassifiedStopCause,
) -> SubagentOutcome {
    let mut outcome = SubagentOutcome::cancelled("task");
    outcome.status = status;
    outcome.output = "durable winner".into();
    outcome
        .extensions
        .insert("agent_id".into(), serde_json::json!("winner"));
    outcome
        .extensions
        .insert("elapsed_ms".into(), serde_json::json!(0));
    outcome.extensions.insert(
        "failure_cause".into(),
        serde_json::json!({"class": cause.class, "operation": cause.operation, "summary": cause.summary}),
    );
    serde_json::from_slice(&serde_json::to_vec(&outcome).unwrap()).unwrap()
}

fn host_outcome(
    failure_cause: Option<tinyagents_harness::no_progress::ClassifiedStopCause>,
    status: SubagentRunStatus,
) -> SubagentRunOutcome {
    SubagentRunOutcome {
        task_id: "task".into(),
        agent_id: "cached-agent".into(),
        output: "cached local result".into(),
        iterations: 3,
        failure_cause,
        elapsed: std::time::Duration::from_millis(10),
        mode: crate::agent::subagent_host::SubagentMode::Typed,
        status,
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        persistence_disposition: SubagentPersistenceDisposition::TerminalInserted,
    }
}

fn project(
    outcome: SubagentOutcome,
    host: Option<SubagentRunOutcome>,
    disposition: SubagentPersistenceDisposition,
    persistence: &OpenHumanPersistence,
) -> SubagentRunOutcome {
    outcome_to_host(
        SubagentRunResult::new(outcome, disposition),
        host,
        persistence,
        &key(),
        "fallback-agent",
    )
}

#[test]
fn durable_winner_cause_is_restored_without_a_host_cache() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().join("checkpoints"));
    let winner_cause = cause("not_found", "file_read", "path was not found");
    let outcome = neutral_outcome(
        SubagentOutcomeKind::Incomplete(SubagentIncomplete::new("path was not found")),
        &winner_cause,
    );

    let projected = project(
        outcome,
        None,
        SubagentPersistenceDisposition::TerminalExisting,
        &persistence,
    );

    assert_eq!(projected.failure_cause, Some(winner_cause));
    assert_eq!(projected.output, "durable winner");
}

#[test]
fn a_losing_cached_cause_cannot_replace_the_durable_winner_cause() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().join("checkpoints"));
    let winner_cause = cause("not_found", "file_read", "path was not found");
    let outcome = neutral_outcome(
        SubagentOutcomeKind::Incomplete(SubagentIncomplete::new("path was not found")),
        &winner_cause,
    );
    let loser = host_outcome(
        Some(cause("permission", "shell", "loser was denied")),
        SubagentRunStatus::Incomplete {
            reason: "loser was denied".into(),
            failure_cause: None,
        },
    );

    let projected = project(
        outcome,
        Some(loser),
        SubagentPersistenceDisposition::TerminalExisting,
        &persistence,
    );

    assert_eq!(projected.failure_cause, Some(winner_cause));
    assert_eq!(projected.output, "durable winner");
}

#[test]
fn ordinary_host_projection_keeps_the_winning_cause() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().join("checkpoints"));
    let winner_cause = cause("permission", "shell", "permission denied");
    let outcome = neutral_outcome(
        SubagentOutcomeKind::Incomplete(SubagentIncomplete::new("permission denied")),
        &winner_cause,
    );
    let host = host_outcome(
        Some(winner_cause.clone()),
        SubagentRunStatus::Incomplete {
            reason: "permission denied".into(),
            failure_cause: None,
        },
    );

    let projected = project(
        outcome,
        Some(host),
        SubagentPersistenceDisposition::TerminalInserted,
        &persistence,
    );

    assert_eq!(projected.failure_cause, Some(winner_cause));
}

#[test]
fn non_incomplete_winners_discard_stale_typed_causes() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().join("checkpoints"));
    let stale = cause("not_found", "file_read", "stale cause");
    let statuses = [
        (SubagentOutcomeKind::Completed, SubagentRunStatus::Completed),
        (
            SubagentOutcomeKind::AwaitingInput(SubagentPause {
                reason: "question".into(),
                resume: SubagentResume::default(),
            }),
            SubagentRunStatus::AwaitingUser {
                question: "question".into(),
                options: None,
                checkpoint: None,
            },
        ),
        (SubagentOutcomeKind::Cancelled, SubagentRunStatus::Cancelled),
    ];

    for (neutral_status, host_status) in statuses {
        let outcome = neutral_outcome(neutral_status, &stale);
        let no_cache = project(
            outcome.clone(),
            None,
            SubagentPersistenceDisposition::TerminalExisting,
            &persistence,
        );
        assert_eq!(no_cache.failure_cause, None);

        let stale_host = host_outcome(Some(stale.clone()), host_status);
        let cached = project(
            outcome,
            Some(stale_host),
            SubagentPersistenceDisposition::TerminalInserted,
            &persistence,
        );
        assert_eq!(cached.failure_cause, None);
    }
}
