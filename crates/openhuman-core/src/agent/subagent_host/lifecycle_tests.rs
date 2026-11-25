use crate::agent::subagent_host::types::SubagentCheckpointData;
use std::sync::Arc;
use tinyagents_orchestration::subagent::{
    PersistedSubagentPause, SubagentOutcome, SubagentOutcomeKind, SubagentPause,
    SubagentPausePersistenceDisposition, SubagentPersistence, SubagentResume, SubagentTaskKey,
};

use super::lifecycle::{
    decode_driver_job_result, load_subagent_checkpoint, merged_resume_history,
    OpenHumanDriverBundle, OpenHumanPersistence,
};
use super::{SubagentMode, SubagentRunError, SubagentRunOutcome, SubagentRunStatus, SubagentUsage};

fn key() -> SubagentTaskKey {
    SubagentTaskKey {
        root_run_id: "root".into(),
        parent_run_id: "parent".into(),
        thread_id: Some("thread".into()),
        task_id: "task".into(),
    }
}

fn cached_outcome(status: SubagentRunStatus) -> SubagentRunOutcome {
    SubagentRunOutcome {
        task_id: "task".into(),
        agent_id: "researcher".into(),
        output: "partial output".into(),
        iterations: 1,
        failure_cause: None,
        elapsed: std::time::Duration::from_millis(1),
        mode: SubagentMode::Typed,
        status,
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::PauseCommitted,
    }
}

fn lifecycle_error(status: &str, task_id: &str) -> tinytools::ToolResult {
    tinytools::ToolResult::error(
        serde_json::json!({"task_id": task_id, "status": status}).to_string(),
    )
}

#[test]
fn driver_error_accepts_only_matching_nonterminal_lifecycle_snapshots() {
    let workspace = tempfile::tempdir().unwrap();
    let bundle = OpenHumanDriverBundle::new(workspace.path().join("checkpoints")).unwrap();
    let task_key = key();
    let public_id = "subagent-job-42";

    for (status, host_status) in [
        (
            "awaiting_input",
            SubagentRunStatus::AwaitingUser {
                question: "Need clarification".into(),
                options: None,
                checkpoint: None,
            },
        ),
        (
            "incomplete",
            SubagentRunStatus::Incomplete {
                reason: "iteration limit".into(),
                failure_cause: None,
            },
        ),
        ("cancelled", SubagentRunStatus::Cancelled),
    ] {
        bundle
            .host_outcome
            .lock()
            .unwrap()
            .insert(task_key.clone(), cached_outcome(host_status));
        let decoded =
            decode_driver_job_result(lifecycle_error(status, public_id), &bundle, &task_key)
                .unwrap();
        assert_eq!(decoded, public_id);
    }
}

#[test]
fn driver_error_does_not_hide_completed_mismatched_or_persistence_failures() {
    let workspace = tempfile::tempdir().unwrap();
    let bundle = OpenHumanDriverBundle::new(workspace.path().join("checkpoints")).unwrap();
    let task_key = key();
    bundle.host_outcome.lock().unwrap().insert(
        task_key.clone(),
        cached_outcome(SubagentRunStatus::AwaitingUser {
            question: "Need clarification".into(),
            options: None,
            checkpoint: None,
        }),
    );

    for result in [
        lifecycle_error("cancelled", "subagent-job-42"),
        lifecycle_error("unknown", "subagent-job-42"),
        tinytools::ToolResult::error("provider transport failed"),
    ] {
        let error = decode_driver_job_result(result, &bundle, &task_key).unwrap_err();
        assert!(error
            .to_string()
            .contains("driver-backed async spawn failed"));
    }

    bundle.failures.lock().unwrap().insert(
        task_key.clone(),
        SubagentRunError::Provider(anyhow::anyhow!("checkpoint write failed")),
    );
    let error = decode_driver_job_result(
        lifecycle_error("awaiting_input", "subagent-job-42"),
        &bundle,
        &task_key,
    )
    .unwrap_err();
    assert!(error.to_string().contains("checkpoint write failed"));

    bundle.failures.lock().unwrap().remove(&task_key);
    bundle.host_outcome.lock().unwrap().insert(
        task_key.clone(),
        cached_outcome(SubagentRunStatus::Completed),
    );
    let error = decode_driver_job_result(
        lifecycle_error("completed", "subagent-job-42"),
        &bundle,
        &task_key,
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("driver-backed async spawn failed"));

    bundle.host_outcome.lock().unwrap().insert(
        task_key.clone(),
        cached_outcome(SubagentRunStatus::AwaitingUser {
            question: "Need clarification".into(),
            options: None,
            checkpoint: None,
        }),
    );
    let error = decode_driver_job_result(lifecycle_error("awaiting_input", ""), &bundle, &task_key)
        .unwrap_err();
    assert!(error.to_string().contains("omitted its registered job id"));
}

fn paused_outcome(question: &str, resume: SubagentResume) -> SubagentOutcome {
    SubagentOutcome {
        task_id: "task".into(),
        worker_thread_id: None,
        tier: None,
        model_override: None,
        output: "partial result".into(),
        history: resume.history.clone(),
        status: SubagentOutcomeKind::AwaitingInput(SubagentPause {
            reason: question.into(),
            resume,
        }),
        usage: Default::default(),
        cost_usd: None,
        cost_source: None,
        extensions: Default::default(),
        artifacts: Vec::new(),
        schema_error: None,
        artifact_error: None,
    }
}

fn scoped_key(root: &str, parent: &str, thread: Option<&str>) -> SubagentTaskKey {
    SubagentTaskKey {
        root_run_id: root.into(),
        parent_run_id: parent.into(),
        thread_id: thread.map(str::to_owned),
        task_id: "task".into(),
    }
}

fn persisted_pause(
    key: SubagentTaskKey,
    question: &str,
    replaces: Option<SubagentResume>,
) -> PersistedSubagentPause {
    let resume = if replaces.is_some() {
        SubagentResume {
            checkpoint: Some(format!("checkpoint-for-{question}")),
            ..SubagentResume::default()
        }
    } else {
        SubagentResume::default()
    };
    PersistedSubagentPause {
        key,
        outcome: paused_outcome(question, resume),
        replaces,
    }
}

#[test]
fn resume_history_keeps_the_durable_transcript_and_appends_user_answer_once() {
    let merged = merged_resume_history(
        vec![tinyinference_llm::message::Message::assistant("prior work")],
        "the user's answer".into(),
    );
    assert_eq!(
        merged
            .iter()
            .map(tinyinference_llm::message::Message::text)
            .collect::<Vec<_>>(),
        vec!["prior work", "the user's answer"],
    );
}

#[tokio::test]
async fn persistence_hashes_an_oversized_scoped_key_for_filesystem_safety() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().to_path_buf());
    let key = SubagentTaskKey {
        root_run_id: "root".repeat(100),
        parent_run_id: "parent".repeat(100),
        thread_id: Some("thread".repeat(100)),
        task_id: "task".repeat(100),
    };

    persistence
        .save_pause(persisted_pause(key.clone(), "question", None))
        .await
        .unwrap();

    assert!(persistence.load_pause(&key).await.unwrap().is_some());
}

#[tokio::test]
async fn legacy_durable_session_resolves_its_key_only_from_owned_checkpoint() {
    let workspace = tempfile::tempdir().unwrap();
    let checkpoint_dir = workspace.path().join("checkpoints");
    let key = key();
    let store = crate::agent::orchestration::subagent_sessions::SubagentSessionStore::new(
        workspace.path().to_path_buf(),
    );
    let session = crate::agent::orchestration::subagent_sessions::upsert_running(
        &store,
        crate::agent::orchestration::subagent_sessions::SubagentSessionUpsert {
            selector: crate::agent::orchestration::subagent_sessions::SubagentSessionSelector {
                parent_session: "owner-session".into(),
                parent_thread_id: Some("thread".into()),
                agent_id: "researcher".into(),
                model: None,
                sandbox_mode: "workspace".into(),
                action_root: None,
                task_key: "task".into(),
            },
            display_name: None,
            task_title: "Task".into(),
            worker_thread_id: None,
            task_id: "task".into(),
            lifecycle_key: None,
            parent_epoch: None,
        },
        None,
    )
    .unwrap();
    OpenHumanPersistence::new(checkpoint_dir.clone())
        .save_pause(persisted_pause(key.clone(), "question", None))
        .await
        .unwrap();

    let resolved = crate::agent::orchestration::subagent_sessions::resolve_lifecycle_key(
        &store,
        &session.subagent_session_id,
        "owner-session",
        Some("thread"),
        &checkpoint_dir,
    )
    .unwrap();
    assert_eq!(resolved, key);
    assert!(
        crate::agent::orchestration::subagent_sessions::resolve_lifecycle_key(
            &store,
            &session.subagent_session_id,
            "different-owner",
            Some("thread"),
            &checkpoint_dir,
        )
        .is_err()
    );
}

#[tokio::test]
async fn tiny_session_port_recovers_legacy_key_from_owned_pause_and_hides_foreign_rows() {
    use tinyagents_orchestration::subagent::SubagentSessionStore as _;

    let workspace = tempfile::tempdir().unwrap();
    crate::storage::local::use_persistent_default_for_test(workspace.path());
    let checkpoint_dir = workspace.path().join(".openhuman/subagent_checkpoints");
    let key = key();
    let store = crate::agent::orchestration::subagent_sessions::SubagentSessionStore::new(
        workspace.path().to_path_buf(),
    );
    crate::agent::orchestration::subagent_sessions::upsert_running(
        &store,
        crate::agent::orchestration::subagent_sessions::SubagentSessionUpsert {
            selector: crate::agent::orchestration::subagent_sessions::SubagentSessionSelector {
                parent_session: "owner-session".into(),
                parent_thread_id: Some("thread".into()),
                agent_id: "researcher".into(),
                model: None,
                sandbox_mode: "workspace".into(),
                action_root: None,
                task_key: "task".into(),
            },
            display_name: None,
            task_title: "Task".into(),
            worker_thread_id: None,
            task_id: "task".into(),
            lifecycle_key: None,
            parent_epoch: None,
        },
        None,
    )
    .unwrap();
    OpenHumanPersistence::new(checkpoint_dir)
        .save_pause(persisted_pause(key.clone(), "question", None))
        .await
        .unwrap();

    let port =
        crate::agent::orchestration::subagent_sessions::OpenHumanTinySubagentSessionStore::new(
            workspace.path().to_path_buf(),
            "owner-session",
            Some("thread".into()),
        );
    let owned = port
        .list_for_parent("owner-session", Some("thread"))
        .await
        .unwrap();
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].lifecycle_key, Some(key));
    assert!(port
        .list_for_parent("foreign-session", Some("thread"))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn persistence_commits_one_scoped_pause_and_recovers_its_original_key() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().to_path_buf());
    let key = key();
    let pause = PersistedSubagentPause {
        key: key.clone(),
        outcome: paused_outcome("need input", SubagentResume::default()),
        replaces: None,
    };

    assert_eq!(
        persistence.save_pause(pause.clone()).await.unwrap(),
        SubagentPausePersistenceDisposition::Inserted
    );
    assert_eq!(
        persistence.save_pause(pause).await.unwrap(),
        SubagentPausePersistenceDisposition::Existing
    );
    assert_eq!(
        OpenHumanPersistence::recover_key(directory.path(), "task").unwrap(),
        key
    );
    assert_eq!(
        load_subagent_checkpoint(directory.path(), "task")
            .unwrap()
            .task_key,
        Some(key)
    );
}

#[tokio::test]
async fn persistence_inserts_one_terminal_and_returns_the_durable_winner() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().to_path_buf());
    let key = key();
    let mut winner = SubagentOutcome::cancelled("task");
    winner.status = SubagentOutcomeKind::Completed;
    winner.output = "winner".into();
    let mut loser = SubagentOutcome::cancelled("task");
    loser.status = SubagentOutcomeKind::Completed;
    loser.output = "loser".into();

    assert_eq!(
        persistence
            .record_terminal(&key, &winner, None)
            .await
            .unwrap(),
        tinyagents_orchestration::subagent::SubagentTerminalPersistenceDisposition::Inserted
    );
    assert_eq!(
        persistence
            .record_terminal(&key, &loser, None)
            .await
            .unwrap(),
        tinyagents_orchestration::subagent::SubagentTerminalPersistenceDisposition::Existing
    );
    assert_eq!(persistence.load_terminal(&key).await.unwrap(), Some(winner));
}

#[tokio::test]
async fn canonical_terminal_hides_legacy_pause_without_rewriting_legacy_files() {
    let directory = tempfile::tempdir().unwrap();
    let key = key();
    let legacy = OpenHumanPersistence::new(directory.path().to_path_buf());
    let checkpoint = SubagentCheckpointData {
        task_key: Some(key.clone()),
        task_id: key.task_id.clone(),
        agent_id: "researcher".into(),
        worker_thread_id: None,
        history: Vec::new(),
        question: "legacy question".into(),
        options: Some(vec!["continue".into()]),
        skill_filter_override: None,
        model_override: None,
        created_at: "2026-10-01T00:00:00Z".into(),
        resume_checkpoint: None,
        resume_metadata: Default::default(),
        output: "legacy partial".into(),
        iterations: 3,
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        elapsed_ms: 12,
    };
    let pause_path = legacy.legacy_pause_path(&key);
    let index_path = legacy.index_path(&key.task_id);
    std::fs::create_dir_all(pause_path.parent().unwrap()).unwrap();
    let pause_bytes = serde_json::to_vec_pretty(&checkpoint).unwrap();
    let index_bytes = serde_json::to_vec(&serde_json::json!({"keys": [key]})).unwrap();
    std::fs::write(&pause_path, &pause_bytes).unwrap();
    std::fs::write(&index_path, &index_bytes).unwrap();

    let mut terminal = SubagentOutcome::cancelled("task");
    terminal.status = SubagentOutcomeKind::Completed;
    terminal.output = "canonical terminal winner".into();
    OpenHumanPersistence::new(directory.path().to_path_buf())
        .record_terminal(&key, &terminal, None)
        .await
        .unwrap();

    assert!(load_subagent_checkpoint(directory.path(), &key.task_id).is_err());
    assert_eq!(std::fs::read(&pause_path).unwrap(), pause_bytes);
    assert_eq!(std::fs::read(&index_path).unwrap(), index_bytes);
}

#[tokio::test]
async fn legacy_pause_migration_preserves_host_checkpoint_metadata_and_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let legacy_key = key();
    let legacy = OpenHumanPersistence::new(directory.path().to_path_buf());
    let checkpoint = SubagentCheckpointData {
        task_key: Some(legacy_key.clone()),
        task_id: legacy_key.task_id.clone(),
        agent_id: "legacy-agent".into(),
        worker_thread_id: Some("worker-thread".into()),
        history: vec![
            tinyagents_session::transcript::TranscriptMessage::assistant("legacy history"),
        ],
        question: "legacy question".into(),
        options: Some(vec!["yes".into(), "no".into()]),
        skill_filter_override: Some("docs".into()),
        model_override: Some("model-x".into()),
        created_at: "2026-10-01T00:00:00Z".into(),
        resume_checkpoint: Some("legacy-checkpoint".into()),
        resume_metadata: Default::default(),
        output: "legacy partial output".into(),
        iterations: 5,
        usage: SubagentUsage {
            input_tokens: 13,
            output_tokens: 7,
            cached_input_tokens: 2,
            charged_amount_usd: 0.75,
            cost_source: tinyagents_harness::cost::CostSource::Estimated,
        },
        artifact_paths: vec!["artifacts/report.md".into()],
        elapsed_ms: 321,
    };
    let pause_path = legacy.legacy_pause_path(&legacy_key);
    std::fs::create_dir_all(pause_path.parent().unwrap()).unwrap();
    let pause_bytes = serde_json::to_vec_pretty(&checkpoint).unwrap();
    std::fs::write(&pause_path, &pause_bytes).unwrap();

    let migrated = OpenHumanPersistence::new(directory.path().to_path_buf());
    assert!(migrated.load_pause(&legacy_key).await.unwrap().is_some());
    let recovered = load_subagent_checkpoint(directory.path(), &legacy_key.task_id).unwrap();

    assert_eq!(recovered.agent_id, checkpoint.agent_id);
    assert_eq!(recovered.worker_thread_id, checkpoint.worker_thread_id);
    assert_eq!(recovered.question, checkpoint.question);
    assert_eq!(recovered.options, checkpoint.options);
    assert_eq!(
        recovered.skill_filter_override,
        checkpoint.skill_filter_override
    );
    assert_eq!(recovered.model_override, checkpoint.model_override);
    assert_eq!(recovered.created_at, checkpoint.created_at);
    assert_eq!(recovered.output, checkpoint.output);
    assert_eq!(recovered.iterations, checkpoint.iterations);
    assert_eq!(recovered.usage, checkpoint.usage);
    assert_eq!(recovered.artifact_paths, checkpoint.artifact_paths);
    assert_eq!(recovered.elapsed_ms, checkpoint.elapsed_ms);
    assert_eq!(std::fs::read(&pause_path).unwrap(), pause_bytes);

    let mut newer_checkpoint = checkpoint;
    let mut newer_key = key();
    newer_key.task_id = "task-with-new-resume-metadata".into();
    newer_checkpoint.task_key = Some(newer_key.clone());
    newer_checkpoint.task_id = newer_key.task_id.clone();
    newer_checkpoint
        .resume_metadata
        .insert("skill_filter_override".into(), "new-skill".into());
    newer_checkpoint
        .resume_metadata
        .insert("model_override".into(), "new-model".into());
    let newer_path = migrated.legacy_pause_path(&newer_key);
    std::fs::create_dir_all(newer_path.parent().unwrap()).unwrap();
    std::fs::write(
        &newer_path,
        serde_json::to_vec_pretty(&newer_checkpoint).unwrap(),
    )
    .unwrap();
    let newer_store = OpenHumanPersistence::new(directory.path().to_path_buf());
    assert!(newer_store.load_pause(&newer_key).await.unwrap().is_some());
    let newer = load_subagent_checkpoint(directory.path(), &newer_key.task_id).unwrap();
    assert_eq!(newer.skill_filter_override.as_deref(), Some("new-skill"));
    assert_eq!(newer.model_override.as_deref(), Some("new-model"));
}

#[tokio::test]
async fn legacy_pause_migration_does_not_return_a_stale_pause_after_terminal_commit() {
    let directory = tempfile::tempdir().unwrap();
    let key = key();
    let persistence = OpenHumanPersistence::new(directory.path().to_path_buf());
    let mut terminal = SubagentOutcome::cancelled("task");
    terminal.status = SubagentOutcomeKind::Completed;
    terminal.output = "terminal winner".into();
    persistence
        .record_terminal(&key, &terminal, None)
        .await
        .unwrap();

    let stale_pause = SubagentCheckpointData {
        task_key: Some(key.clone()),
        task_id: key.task_id.clone(),
        agent_id: "legacy-agent".into(),
        worker_thread_id: None,
        history: Vec::new(),
        question: "stale pause".into(),
        options: None,
        skill_filter_override: None,
        model_override: None,
        created_at: "2026-10-01T00:00:00Z".into(),
        resume_checkpoint: None,
        resume_metadata: Default::default(),
        output: "stale output".into(),
        iterations: 1,
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        elapsed_ms: 1,
    };

    let migrated = persistence
        .migrate_legacy_pause(&key, stale_pause)
        .await
        .unwrap();
    assert!(
        migrated.is_none(),
        "terminal CAS winner must close the stale pause"
    );
}

#[path = "lifecycle_terminal_tests.rs"]
mod terminal_tests;
