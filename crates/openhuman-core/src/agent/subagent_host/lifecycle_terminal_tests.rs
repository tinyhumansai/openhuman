use super::*;

#[tokio::test]
async fn driver_cancel_uses_the_typed_terminal_for_lossy_idle_sessions() {
    use crate::agent::orchestration::subagent_sessions::{
        cancel_driver_job_by_task, mark_finished, upsert_running, DurableSubagentStatus,
        SubagentSessionSelector, SubagentSessionStore, SubagentSessionUpsert,
    };

    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path();
    crate::storage::local::use_persistent_default_for_test(workspace);
    let store = SubagentSessionStore::new(workspace.to_path_buf());
    let jobs = tinyagents_orchestration::subagent::SubAgentJobRegistry::default();
    let persistence = OpenHumanPersistence::new(workspace.join(".openhuman/subagent_checkpoints"));

    for (task_id, terminal, expected) in [
        (
            "typed-completed",
            SubagentOutcomeKind::Completed,
            Some(tinyagents_orchestration::subagent::FinishedOutcome::Completed),
        ),
        (
            "typed-incomplete",
            SubagentOutcomeKind::Incomplete(
                tinyagents_orchestration::subagent::SubagentIncomplete::new("iteration cap"),
            ),
            Some(tinyagents_orchestration::subagent::FinishedOutcome::Failed),
        ),
    ] {
        let key = SubagentTaskKey {
            root_run_id: "root-typed-finished".into(),
            parent_run_id: "parent-typed-finished".into(),
            thread_id: Some("thread-typed-finished".into()),
            task_id: task_id.into(),
        };
        let session = upsert_running(
            &store,
            SubagentSessionUpsert {
                selector: SubagentSessionSelector {
                    parent_session: "parent-typed-finished".into(),
                    parent_thread_id: key.thread_id.clone(),
                    agent_id: "researcher".into(),
                    model: None,
                    sandbox_mode: "read_only".into(),
                    action_root: None,
                    task_key: task_id.into(),
                },
                display_name: None,
                task_title: task_id.into(),
                worker_thread_id: None,
                task_id: task_id.into(),
                lifecycle_key: Some(key.clone()),
                parent_epoch: None,
            },
            None,
        )
        .unwrap();
        let mut outcome = SubagentOutcome::cancelled(task_id);
        outcome.status = terminal;
        persistence
            .record_terminal(&key, &outcome, None)
            .await
            .unwrap();
        mark_finished(
            &store,
            &session.subagent_session_id,
            task_id,
            &match task_id {
                "typed-completed" => SubagentRunStatus::Completed,
                _ => SubagentRunStatus::Incomplete {
                    reason: "iteration cap".into(),
                    failure_cause: None,
                },
            },
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            store.load().unwrap()[0].status,
            DurableSubagentStatus::Idle,
            "both typed terminal outcomes share the legacy Idle session state"
        );

        let selected = cancel_driver_job_by_task(&store, workspace, &jobs, task_id, None)
            .unwrap()
            .expect("typed terminal remains reportable");
        assert_eq!(selected.already_finished, expected, "{task_id}");
    }
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
async fn shared_flights_are_partitioned_by_agent_and_workspace() {
    use crate::core::runtime::agent_scope::test_agent_context;
    use crate::core::runtime::{context::CoreContext, DomainSet};

    let root = CoreContext::for_test(DomainSet::full(), None);
    let alpha = test_agent_context(&root, "alpha");
    let beta = test_agent_context(&root, "beta");
    let key = key();
    let alpha_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&alpha));
    let beta_ctx = crate::core::runtime::DispatchCtx::new(beta);

    let alpha_workspace_a = super::super::lifecycle::shared_subagent_flight(
        &alpha_ctx,
        std::path::Path::new("/workspace-a"),
    );
    let alpha_workspace_b = super::super::lifecycle::shared_subagent_flight(
        &alpha_ctx,
        std::path::Path::new("/workspace-b"),
    );
    let alpha_workspace_a_again = super::super::lifecycle::shared_subagent_flight(
        &alpha_ctx,
        std::path::Path::new("/workspace-a"),
    );
    let beta_workspace_a = super::super::lifecycle::shared_subagent_flight(
        &beta_ctx,
        std::path::Path::new("/workspace-a"),
    );

    assert!(alpha_workspace_a.claim(key.clone()).1);
    assert!(!alpha_workspace_a_again.claim(key.clone()).1);
    assert!(alpha_workspace_b.claim(key.clone()).1);
    assert!(beta_workspace_a.claim(key).1);
}
