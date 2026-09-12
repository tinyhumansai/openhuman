use super::*;
use crate::openhuman::agent::tinyagents::orchestration::{
    openhuman_steering_handle, OrchestrationTaskStatus, SteeringHandle, SteeringPolicy,
    SteeringRunClass,
};
use std::sync::MutexGuard;

/// Serializes every test that touches the global [`REGISTRY`]. We reuse the
/// crate-wide `TEST_ENV_LOCK` (rather than a module-local mutex) because the
/// destructive `cancel_all` path is also reachable from the `threads::ops`
/// tests — those hold the same lock, so this prevents a purge there from
/// wiping entries a test here is mid-way through.
fn test_guard() -> MutexGuard<'static, ()> {
    // Recover from a poisoned guard so one panicking test doesn't cascade.
    crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn dummy_abort() -> AbortHandle {
    tokio::spawn(async {}).abort_handle()
}

/// Per-process-run unique workspace for the detached task store.
///
/// The task store is now a durable JSONL file under the given workspace
/// (issue #4249). Pointing tests at the shared `std::env::temp_dir()` leaked
/// records **across** test-process runs: a `task-ledger-1` left `Completed`
/// by a prior run would make `record_spawned`'s insert/`mark_running` no-op
/// (id already terminal), so this run would observe the stale terminal status
/// instead of `Running`. A fresh temp dir per process run keeps the store
/// hermetic; task ids are unique across tests so a single shared dir is safe
/// within a run. The `TempDir` lives for the whole process (cleaned at exit).
fn test_workspace() -> PathBuf {
    static WORKSPACE: std::sync::LazyLock<tempfile::TempDir> = std::sync::LazyLock::new(|| {
        tempfile::tempdir().expect("create hermetic test task-store workspace")
    });
    WORKSPACE.path().to_path_buf()
}

/// Register a sub-agent for tests, returning the status sender so the test
/// can drive completion. Keeping the sender alive keeps the channel open.
fn register_test(
    task_id: &str,
    parent_session: &str,
    rq: Arc<RunQueue>,
) -> watch::Sender<SubagentStatus> {
    register_test_with_thread(task_id, parent_session, None, rq)
}

/// Like [`register_test`] but lets a test set the parent thread id so it can
/// exercise [`cancel_for_thread`].
fn register_test_with_thread(
    task_id: &str,
    parent_session: &str,
    parent_thread_id: Option<&str>,
    rq: Arc<RunQueue>,
) -> watch::Sender<SubagentStatus> {
    let (tx, rx) = status_channel();
    register(
        task_id.into(),
        "researcher".into(),
        parent_session.into(),
        None,
        None,
        test_workspace(),
        parent_thread_id.map(Into::into),
        rq,
        dummy_abort(),
        rx,
    );
    tx
}

#[tokio::test]
async fn task_store_records_spawn_complete_and_cancel() {
    let _guard = test_guard();
    // Spawn → the ledger sees a running SubAgent task scoped to the parent.
    let tx = register_test("task-ledger-1", "ledger-parent", RunQueue::new());
    let running = task_records(Some("ledger-parent"));
    assert!(
        running
            .iter()
            .any(|r| r.spec.task_id.as_str() == "task-ledger-1"
                && r.spec.parent_run_id.as_ref().map(|id| id.as_str()) == Some("ledger-parent")
                && r.spec.root_run_id.as_ref().map(|id| id.as_str()) == Some("ledger-parent")
                && r.spec.timeout_ms == Some(DETACHED_LEDGER_TIMEOUT_MS)
                && r.status == OrchestrationTaskStatus::Running),
        "spawned sub-agent is recorded Running: {running:?}"
    );

    // Publish a terminal status → the watcher mirrors Completed into the store.
    tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 2,
    })
    .unwrap();
    // Let the watcher task observe the change.
    for _ in 0..50 {
        tokio::task::yield_now().await;
        if task_records(None)
            .iter()
            .any(|r| r.spec.task_id.as_str() == "task-ledger-1" && r.is_terminal())
        {
            break;
        }
    }
    let after = task_records(None);
    let rec = after
        .iter()
        .find(|r| r.spec.task_id.as_str() == "task-ledger-1")
        .expect("ledger record present");
    assert_eq!(rec.status, OrchestrationTaskStatus::Completed);

    // A second sub-agent that gets cancelled is recorded Cancelled.
    let _tx2 = register_test("task-ledger-2", "ledger-parent", RunQueue::new());
    assert!(cancel_by_task("task-ledger-2").is_some());
    let cancelled = task_records(None)
        .into_iter()
        .find(|r| r.spec.task_id.as_str() == "task-ledger-2")
        .expect("cancelled record present");
    assert_eq!(cancelled.status, OrchestrationTaskStatus::Cancelled);

    prune("task-ledger-1");
}

#[tokio::test]
async fn task_id_for_session_enforces_parent_ownership() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let (tx, rx) = status_channel();
    register(
        "task-session".into(),
        "researcher".into(),
        "session-owner".into(),
        None,
        Some("subsess-1".into()),
        test_workspace(),
        Some("thread-1".into()),
        rq,
        dummy_abort(),
        rx,
    );

    assert_eq!(
        task_id_for_session("subsess-1", "session-owner").unwrap(),
        "task-session"
    );
    assert!(matches!(
        task_id_for_session("subsess-1", "session-other"),
        Err(WaitError::NotOwned)
    ));
    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-session");
}

#[tokio::test]
async fn snapshot_and_block_scope_to_parent_and_reflect_live_status() {
    let _guard = test_guard();
    let (tx_a, rx_a) = status_channel();
    register(
        "task-fleet-a".into(),
        "researcher".into(),
        "fleet-parent".into(),
        None,
        Some("subsess-a".into()),
        test_workspace(),
        None,
        RunQueue::new(),
        dummy_abort(),
        rx_a,
    );
    let (tx_b, rx_b) = status_channel();
    register(
        "task-fleet-b".into(),
        "code_executor".into(),
        "fleet-parent".into(),
        None,
        Some("subsess-b".into()),
        test_workspace(),
        None,
        RunQueue::new(),
        dummy_abort(),
        rx_b,
    );
    // A worker owned by a different parent must not leak into the snapshot.
    let (tx_other, rx_other) = status_channel();
    register(
        "task-fleet-other".into(),
        "researcher".into(),
        "other-parent".into(),
        None,
        Some("subsess-other".into()),
        test_workspace(),
        None,
        RunQueue::new(),
        dummy_abort(),
        rx_other,
    );

    // `b` pauses awaiting the user; `a` stays running.
    tx_b.send(SubagentStatus::AwaitingUser {
        question: "which repo?".into(),
    })
    .unwrap();

    let snap = snapshot_for_parent("fleet-parent");
    assert_eq!(snap.len(), 2, "only this parent's workers: {snap:?}");
    // Ordered by agent_id then task_id → code_executor before researcher.
    assert_eq!(snap[0].agent_id, "code_executor");
    assert_eq!(snap[0].status, "awaiting_user");
    assert_eq!(snap[1].agent_id, "researcher");
    assert_eq!(snap[1].status, "running");

    let block =
        active_subagents_context_block("fleet-parent", &test_workspace()).expect("block present");
    assert!(block.contains("[active_subagents]"));
    assert!(block.contains("You have 2 sub-agent worker(s)"));
    assert!(block.contains("session=subsess-a"));
    assert!(block.contains("session=subsess-b · task=task-fleet-b · status=awaiting_user"));
    assert!(block.ends_with("[/active_subagents]\n\n"));

    // A parent with no registered workers gets no block (no perturbation).
    assert!(active_subagents_context_block("nobody-here", &test_workspace()).is_none());

    // Durable-store fallback: a session persisted by an EARLIER turn /
    // process lifetime (empty live registry for this parent) must still
    // surface in the roster, so a cold-booted orchestrator can resume by
    // subagent_session_id instead of re-delegating from scratch.
    {
        use crate::openhuman::agent::harness::subagent_runner::SubagentRunStatus;
        use crate::openhuman::agent::orchestration::subagent_sessions::{
            self, SubagentSessionSelector, SubagentSessionStore, SubagentSessionUpsert,
        };
        let durable_ws = tempfile::tempdir().expect("durable roster tempdir");
        let store = SubagentSessionStore {
            workspace_dir: durable_ws.path().to_path_buf(),
        };
        let session = subagent_sessions::upsert_running(
            &store,
            SubagentSessionUpsert {
                selector: SubagentSessionSelector {
                    parent_session: "cold-parent".into(),
                    parent_thread_id: Some("thread-cold".into()),
                    agent_id: "workflow_builder".into(),
                    toolkit: None,
                    model: None,
                    sandbox_mode: "None".into(),
                    action_root: None,
                    task_key: "daily-x-trending".into(),
                },
                display_name: Some("Workflow Builder".into()),
                task_title: "Daily X trending email workflow".into(),
                worker_thread_id: None,
                task_id: "task-cold-1".into(),
            },
            None,
        )
        .expect("upsert durable session");
        subagent_sessions::mark_finished(
            &store,
            &session.subagent_session_id,
            "task-cold-1",
            &SubagentRunStatus::Completed,
            Vec::new(),
        )
        .expect("mark idle");

        let block = active_subagents_context_block("cold-parent", durable_ws.path())
            .expect("durable-only roster present");
        assert!(block.contains(&format!("session={}", session.subagent_session_id)));
        assert!(block.contains("status=idle"));
        assert!(block.contains("about: Daily X trending email workflow"));
        // Other parents' durable sessions must not leak in.
        assert!(active_subagents_context_block("unrelated-parent", durable_ws.path()).is_none());
    }

    let _ = tx_a.send(SubagentStatus::Completed {
        output: "x".into(),
        iterations: 1,
    });
    let _ = tx_other.send(SubagentStatus::Completed {
        output: "x".into(),
        iterations: 1,
    });
    prune("task-fleet-a");
    prune("task-fleet-b");
    prune("task-fleet-other");
}

#[tokio::test]
async fn resume_ref_for_task_includes_resume_fields_and_enforces_ownership() {
    let _guard = test_guard();
    let (tx, rx) = status_channel();
    register(
        "task-resume".into(),
        "researcher".into(),
        "session-owner".into(),
        None,
        Some("subsess-resume".into()),
        test_workspace(),
        Some("thread-1".into()),
        RunQueue::new(),
        dummy_abort(),
        rx,
    );

    let reference = resume_ref_for_task("task-resume", "session-owner").expect("resume reference");
    assert_eq!(reference.task_id, "task-resume");
    assert_eq!(reference.agent_id, "researcher");
    assert_eq!(
        reference.subagent_session_id.as_deref(),
        Some("subsess-resume")
    );
    assert!(matches!(
        resume_ref_for_task("task-resume", "session-other"),
        Err(WaitError::NotOwned)
    ));

    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-resume");
}

#[tokio::test]
async fn task_id_for_session_prefers_live_task_over_terminal_task() {
    let _guard = test_guard();
    let (old_tx, old_rx) = status_channel();
    register(
        "task-old".into(),
        "researcher".into(),
        "session-owner".into(),
        None,
        Some("subsess-live".into()),
        test_workspace(),
        Some("thread-1".into()),
        RunQueue::new(),
        dummy_abort(),
        old_rx,
    );
    let _ = old_tx.send(SubagentStatus::Completed {
        output: "old".into(),
        iterations: 1,
    });
    let (_new_tx, new_rx) = status_channel();
    register(
        "task-new".into(),
        "researcher".into(),
        "session-owner".into(),
        None,
        Some("subsess-live".into()),
        test_workspace(),
        Some("thread-1".into()),
        RunQueue::new(),
        dummy_abort(),
        new_rx,
    );

    assert_eq!(
        task_id_for_session("subsess-live", "session-owner").unwrap(),
        "task-new"
    );
    prune("task-old");
    prune("task-new");
}

#[tokio::test]
async fn steer_pushes_into_the_subagent_queue() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let tx = register_test("task-steer", "session-A", rq.clone());

    steer(
        "task-steer",
        "session-A",
        "refocus on memory safety".into(),
        QueueMode::Steer,
    )
    .await
    .expect("steer should succeed");

    let status = rq.status().await;
    assert_eq!(status.steers, 1, "steer should land in the steer lane");

    // collect mode goes to the collect lane
    steer(
        "task-steer",
        "session-A",
        "extra context".into(),
        QueueMode::Collect,
    )
    .await
    .unwrap();
    assert_eq!(rq.status().await.collects, 1);

    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-steer");
}

#[tokio::test]
async fn steer_prefers_registered_tinyagents_handle() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let tx = register_test("task-registered-steer", "session-A", rq.clone());
    let handle = SteeringHandle::allow_all();
    let task_id = TaskId::new("task-registered-steer");
    shared_steering_registry().register(task_id.clone(), handle.clone());

    steer(
        "task-registered-steer",
        "session-A",
        "refocus".into(),
        QueueMode::Steer,
    )
    .await
    .expect("steer should succeed");

    let status = rq.status().await;
    assert_eq!(status.steers, 0, "registered handle bypasses RunQueue");
    let commands = handle.drain();
    assert_eq!(commands.len(), 1);
    match &commands[0] {
        SteeringCommand::InjectMessage(message) => {
            assert_eq!(message.text(), "[User steering message]: refocus");
        }
        other => panic!("expected injected steering message, got {other:?}"),
    }

    let _ = shared_steering_registry().deregister(&task_id);
    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-registered-steer");
}

#[tokio::test]
async fn steer_directive_delivers_control_flow_via_background_policy() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let tx = register_test("task-directive", "session-A", rq.clone());
    // A background sub-agent handle accepts Cancel/Redirect/Resume.
    let handle = openhuman_steering_handle(SteeringRunClass::Background);
    let task_id = TaskId::new("task-directive");
    shared_steering_registry().register(task_id.clone(), handle.clone());

    steer_directive(
        "task-directive",
        "session-A",
        SteeringDirective::Redirect("focus on the failing test".into()),
    )
    .expect("redirect should be accepted");
    steer_directive("task-directive", "session-A", SteeringDirective::Cancel)
        .expect("cancel should be accepted");

    // RunQueue is untouched — directives never fall back to it.
    assert_eq!(rq.status().await.steers, 0);
    let commands = handle.drain();
    assert_eq!(commands.len(), 2);
    assert!(matches!(
        &commands[0],
        SteeringCommand::Redirect { instruction } if instruction == "focus on the failing test"
    ));
    assert_eq!(commands[1], SteeringCommand::Cancel);

    let _ = shared_steering_registry().deregister(&task_id);
    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-directive");
}

#[tokio::test]
async fn steer_directive_refuses_kinds_the_policy_rejects() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let tx = register_test("task-tight", "session-A", rq);
    // An interactive-class handle only allows InjectMessage/Pause, so a
    // Cancel directive must be refused up front rather than enqueued (which
    // would abort the run).
    let handle = SteeringHandle::new(
        SteeringPolicy::new()
            .allow(SteeringCommandKind::InjectMessage)
            .allow(SteeringCommandKind::Pause),
    );
    let task_id = TaskId::new("task-tight");
    shared_steering_registry().register(task_id.clone(), handle.clone());

    assert_eq!(
        steer_directive("task-tight", "session-A", SteeringDirective::Cancel),
        Err(SteerDirectiveError::PolicyRejected)
    );
    // Pause is allowed on the tight policy.
    steer_directive("task-tight", "session-A", SteeringDirective::Pause)
        .expect("pause should be accepted by the tight policy");
    let commands = handle.drain();
    assert_eq!(commands, vec![SteeringCommand::Pause]);

    let _ = shared_steering_registry().deregister(&task_id);
    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-tight");
}

#[tokio::test]
async fn steer_directive_enforces_ownership_and_registration() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let tx = register_test("task-own", "session-owner", rq);

    // Cross-parent is refused before any handle lookup.
    assert_eq!(
        steer_directive("task-own", "session-intruder", SteeringDirective::Resume),
        Err(SteerDirectiveError::NotOwned)
    );
    // Unknown task id.
    assert_eq!(
        steer_directive("task-missing", "session-owner", SteeringDirective::Resume),
        Err(SteerDirectiveError::Unknown)
    );
    // Owned but no registered crate handle → cannot deliver control-flow.
    assert_eq!(
        steer_directive("task-own", "session-owner", SteeringDirective::Resume),
        Err(SteerDirectiveError::NoRegisteredHandle)
    );

    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-own");
}

#[tokio::test]
async fn steer_rejects_cross_parent_and_unknown() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let _tx = register_test("task-owned", "session-owner", rq);

    assert_eq!(
        steer(
            "task-owned",
            "session-intruder",
            "x".into(),
            QueueMode::Steer
        )
        .await,
        Err(SteerError::NotOwned)
    );
    assert_eq!(
        steer(
            "task-missing",
            "session-owner",
            "x".into(),
            QueueMode::Steer
        )
        .await,
        Err(SteerError::Unknown)
    );
    prune("task-owned");
}

#[tokio::test]
async fn steer_after_terminal_is_rejected() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let tx = register_test("task-term", "session-A", rq);
    let _ = tx.send(SubagentStatus::Failed {
        error: "boom".into(),
    });

    assert_eq!(
        steer("task-term", "session-A", "x".into(), QueueMode::Steer).await,
        Err(SteerError::AlreadyDone)
    );
    prune("task-term");
}

#[tokio::test]
async fn wait_returns_completion_once_published() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let tx = register_test("task-wait", "session-A", rq);

    tokio::spawn(async move {
        let _ = tx.send(SubagentStatus::Completed {
            output: "the answer".into(),
            iterations: 3,
        });
        // keep sender alive until after send
        drop(tx);
    });

    let outcome = wait("task-wait", "session-A", Duration::from_secs(5))
        .await
        .expect("wait should resolve");
    match outcome {
        WaitOutcome::Terminal(SubagentStatus::Completed { output, iterations }) => {
            assert_eq!(output, "the answer");
            assert_eq!(iterations, 3);
        }
        other => panic!("expected completed terminal, got {other:?}"),
    }

    // pruned after a terminal wait
    assert!(matches!(
        wait("task-wait", "session-A", Duration::from_millis(10)).await,
        Err(WaitError::Unknown)
    ));
}

#[tokio::test]
async fn wait_times_out_and_leaves_entry_intact() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let _tx = register_test("task-slow", "session-A", rq);

    let outcome = wait("task-slow", "session-A", Duration::from_millis(20))
        .await
        .expect("wait should resolve");
    assert!(matches!(
        outcome,
        WaitOutcome::TimedOut(SubagentStatus::Running)
    ));

    // still steerable after a timed-out wait
    assert!(steer(
        "task-slow",
        "session-A",
        "still here".into(),
        QueueMode::Steer
    )
    .await
    .is_ok());
    prune("task-slow");
}

#[tokio::test]
async fn cancel_for_thread_aborts_only_matching_entries() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let _a = register_test_with_thread("task-tA-1", "session-A", Some("thread-X"), rq.clone());
    let _b = register_test_with_thread("task-tA-2", "session-A", Some("thread-X"), rq.clone());
    // Different thread — must survive.
    let _c = register_test_with_thread("task-tB", "session-A", Some("thread-Y"), rq.clone());
    // Headless (no parent thread) — must survive.
    let _d = register_test_with_thread("task-headless", "session-A", None, rq);

    let cancelled = cancel_for_thread("thread-X");
    assert_eq!(cancelled, 2, "both thread-X entries should be cancelled");

    // The two cancelled entries are gone (steer can't find them).
    assert_eq!(
        steer("task-tA-1", "session-A", "x".into(), QueueMode::Steer).await,
        Err(SteerError::Unknown)
    );
    assert_eq!(
        steer("task-tA-2", "session-A", "x".into(), QueueMode::Steer).await,
        Err(SteerError::Unknown)
    );

    // Non-matching entries stay live and steerable.
    assert!(steer("task-tB", "session-A", "x".into(), QueueMode::Steer)
        .await
        .is_ok());
    assert!(
        steer("task-headless", "session-A", "x".into(), QueueMode::Steer)
            .await
            .is_ok()
    );

    // Idempotent: a second pass cancels nothing.
    assert_eq!(cancel_for_thread("thread-X"), 0);

    prune("task-tB");
    prune("task-headless");
}

#[tokio::test]
async fn cancel_by_task_returns_metadata_and_removes_entry() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let _tx = register_test_with_thread("task-cbt", "session-Z", Some("thread-cbt"), rq.clone());
    let task_id = TaskId::new("task-cbt");
    shared_steering_registry().register(task_id.clone(), SteeringHandle::allow_all());

    let meta = cancel_by_task("task-cbt").expect("known task should cancel");
    assert_eq!(meta.agent_id, "researcher");
    assert_eq!(meta.parent_session, "session-Z");
    assert_eq!(meta.parent_thread_id.as_deref(), Some("thread-cbt"));
    assert!(
        shared_steering_registry().get(&task_id).is_none(),
        "hard cancel should remove the registered steering handle"
    );

    // Entry is gone — steer can no longer find it, and a second cancel is a no-op.
    assert_eq!(
        steer("task-cbt", "session-Z", "x".into(), QueueMode::Steer).await,
        Err(SteerError::Unknown)
    );
    assert!(cancel_by_task("task-cbt").is_none());
    // Unknown ids are simply None.
    assert!(cancel_by_task("never-existed").is_none());
}

#[tokio::test]
async fn cancel_all_clears_everything() {
    let _guard = test_guard();
    let rq = RunQueue::new();
    let _a = register_test_with_thread("task-all-1", "session-A", Some("thread-1"), rq.clone());
    // Headless (no parent thread) — aborted, but contributes no thread id.
    let _b = register_test_with_thread("task-all-2", "session-B", None, rq);

    let cancelled_threads = cancel_all();
    assert!(
        cancelled_threads.contains(&"thread-1".to_string()),
        "cancel_all should report the parent thread of the cancelled sub-agent"
    );
    assert!(
        !cancelled_threads.iter().any(|t| t.is_empty()),
        "headless sub-agents must not contribute an id"
    );

    assert_eq!(
        steer("task-all-1", "session-A", "x".into(), QueueMode::Steer).await,
        Err(SteerError::Unknown)
    );
    assert_eq!(
        steer("task-all-2", "session-B", "x".into(), QueueMode::Steer).await,
        Err(SteerError::Unknown)
    );
    // Registry is empty now.
    assert!(cancel_all().is_empty());
}
