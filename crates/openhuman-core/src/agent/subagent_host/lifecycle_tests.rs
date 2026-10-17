use tinyagents_orchestration::subagent::{
    PersistedSubagentPause, SubagentOutcome, SubagentOutcomeKind, SubagentPause,
    SubagentPausePersistenceDisposition, SubagentPersistence, SubagentResume, SubagentTaskKey,
};

use super::lifecycle::{
    host_in_flight, load_subagent_checkpoint, merged_resume_history, HostInFlight,
    OpenHumanPersistence,
};
use super::{SubagentMode, SubagentRunOutcome, SubagentRunStatus, SubagentUsage};

fn key() -> SubagentTaskKey {
    SubagentTaskKey {
        root_run_id: "root".into(),
        parent_run_id: "parent".into(),
        thread_id: Some("thread".into()),
        task_id: "task".into(),
    }
}

fn paused_outcome(question: &str, resume: SubagentResume) -> SubagentOutcome {
    SubagentOutcome {
        task_id: "task".into(),
        output: "partial result".into(),
        history: resume.history.clone(),
        status: SubagentOutcomeKind::AwaitingInput(SubagentPause {
            reason: question.into(),
            resume,
        }),
        usage: Default::default(),
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
async fn continuation_replaces_only_the_pause_it_loaded_and_stale_duplicate_gets_winner() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().to_path_buf());
    let key = key();
    let original = SubagentResume::default();
    assert_eq!(
        persistence
            .save_pause(persisted_pause(key.clone(), "first question", None))
            .await
            .unwrap(),
        SubagentPausePersistenceDisposition::Inserted
    );

    assert_eq!(
        persistence
            .save_pause(persisted_pause(
                key.clone(),
                "replacement question",
                Some(original.clone()),
            ))
            .await
            .unwrap(),
        SubagentPausePersistenceDisposition::Replaced
    );
    assert_eq!(
        persistence
            .save_pause(persisted_pause(
                key.clone(),
                "stale duplicate question",
                Some(original),
            ))
            .await
            .unwrap(),
        SubagentPausePersistenceDisposition::Existing
    );

    let winner = persistence.load_pause(&key).await.unwrap().unwrap();
    assert!(matches!(
        winner.status,
        SubagentOutcomeKind::AwaitingInput(SubagentPause { ref reason, .. })
            if reason == "replacement question"
    ));
}

#[tokio::test]
async fn task_id_index_retains_scoped_collisions_and_terminal_does_not_reopen_pause() {
    let directory = tempfile::tempdir().unwrap();
    let persistence = OpenHumanPersistence::new(directory.path().to_path_buf());
    let first = scoped_key("root-a", "parent-a", Some("thread-a"));
    let second = scoped_key("root-b", "parent-b", Some("thread-b"));
    assert_eq!(
        persistence
            .save_pause(persisted_pause(first.clone(), "first", None))
            .await
            .unwrap(),
        SubagentPausePersistenceDisposition::Inserted
    );
    assert_eq!(
        persistence
            .save_pause(persisted_pause(second.clone(), "second", None))
            .await
            .unwrap(),
        SubagentPausePersistenceDisposition::Inserted
    );
    assert!(load_subagent_checkpoint(directory.path(), "task").is_err());

    let mut terminal = SubagentOutcome::cancelled("task");
    terminal.status = SubagentOutcomeKind::Completed;
    assert_eq!(
        persistence
            .record_terminal(&first, &terminal, None)
            .await
            .unwrap(),
        tinyagents_orchestration::subagent::SubagentTerminalPersistenceDisposition::PauseExisting
    );
    assert_eq!(
        persistence
            .record_terminal(&first, &terminal, Some(&SubagentResume::default()))
            .await
            .unwrap(),
        tinyagents_orchestration::subagent::SubagentTerminalPersistenceDisposition::Inserted
    );
    assert_eq!(
        load_subagent_checkpoint(directory.path(), "task")
            .unwrap()
            .task_key,
        Some(second.clone())
    );
    assert!(persistence.load_pause(&first).await.unwrap().is_none());
    assert!(persistence.load_terminal(&first).await.unwrap().is_some());
}

#[tokio::test]
async fn terminal_and_pause_race_leave_only_one_authoritative_state() {
    let directory = tempfile::tempdir().unwrap();
    let persistence =
        std::sync::Arc::new(OpenHumanPersistence::new(directory.path().to_path_buf()));
    let key = key();
    let mut terminal = SubagentOutcome::cancelled("task");
    terminal.status = SubagentOutcomeKind::Completed;
    let (pause, terminal_write) = tokio::join!(
        persistence.save_pause(persisted_pause(key.clone(), "question", None)),
        persistence.record_terminal(&key, &terminal, None),
    );
    assert!(pause.is_ok());
    assert_eq!(
        terminal_write.unwrap(),
        tinyagents_orchestration::subagent::SubagentTerminalPersistenceDisposition::PauseExisting
    );
    assert!(persistence.load_terminal(&key).await.unwrap().is_none());
    assert!(persistence.load_pause(&key).await.unwrap().is_some());
    assert!(load_subagent_checkpoint(directory.path(), "task").is_ok());
}

#[tokio::test]
async fn host_coalescer_keeps_the_leader_winner_and_cancels_only_the_observer() {
    let entry = HostInFlight::new();
    let key = key();
    let cancelled = tinyagents_harness::CancellationToken::new();
    cancelled.cancel();

    let observer = entry
        .wait(&cancelled, &key, "observer-agent")
        .await
        .unwrap()
        .expect("cancelled follower reports an outcome");
    assert!(matches!(observer.status, SubagentRunStatus::Cancelled));
    assert_eq!(
        observer.persistence_disposition,
        tinyagents_orchestration::subagent::SubagentPersistenceDisposition::ObserverCancelled
    );

    let winner = SubagentRunOutcome {
        task_id: key.task_id.clone(),
        agent_id: "leader-agent".into(),
        output: "durable winner".into(),
        iterations: 3,
        elapsed: std::time::Duration::from_millis(42),
        mode: SubagentMode::Typed,
        status: SubagentRunStatus::Completed,
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted,
    };
    entry.complete(Some(winner)).await;

    let active = tinyagents_harness::CancellationToken::new();
    let follower = entry
        .wait(&active, &key, "other-agent")
        .await
        .unwrap()
        .expect("leader result remains available");
    assert_eq!(follower.output, "durable winner");
    assert_eq!(follower.agent_id, "leader-agent");
    assert_eq!(follower.elapsed, std::time::Duration::from_millis(42));
    assert_eq!(
        follower.persistence_disposition,
        tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalExisting
    );
}

#[tokio::test]
async fn each_agent_dedupes_its_own_in_flight_sub_agents() {
    use crate::core::runtime::agent_scope::test_agent_context;
    use crate::core::runtime::{context::CoreContext, DomainSet};
    use std::sync::Arc;

    let root = CoreContext::for_test(DomainSet::full(), None);
    let alpha = test_agent_context(&root, "alpha");
    let beta = test_agent_context(&root, "beta");

    let alpha_map = {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&alpha));

        host_in_flight(&scope_ctx)
    };
    let beta_map = {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(beta);

        host_in_flight(&scope_ctx)
    };
    let alpha_again = {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(alpha);

        host_in_flight(&scope_ctx)
    };

    assert!(!Arc::ptr_eq(&alpha_map, &beta_map));
    assert!(Arc::ptr_eq(&alpha_map, &alpha_again));
}
