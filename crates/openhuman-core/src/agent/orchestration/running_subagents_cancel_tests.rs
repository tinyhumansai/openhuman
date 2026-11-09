//! Cancelling and stopping registered sub-agents: by task, by thread, and all.

use super::*;
use tinyagents_orchestration::steering::SteerError;

/// A finished run stays registered until the terminal sweep, so a late
/// "Cancel" still finds it. It must come back flagged, so the RPC does not
/// rewrite a completed session as "cancelled by user" — while a run paused on
/// the user is still a real cancel.
#[tokio::test]
async fn cancel_by_task_flags_a_run_that_already_finished() {
    let _guard = test_guard_async().await;
    let cases = [
        (
            "task-cbt-done",
            DetachedSubagentStatus::Completed {
                output: "ok".into(),
                iterations: 6,
            },
            Some(FinishedOutcome::Completed),
        ),
        (
            "task-cbt-failed",
            DetachedSubagentStatus::Failed {
                error: "boom".into(),
            },
            Some(FinishedOutcome::Failed),
        ),
        (
            "task-cbt-paused",
            DetachedSubagentStatus::AwaitingUser {
                question: "which?".into(),
            },
            None,
        ),
    ];
    for (task_id, status, finished) in cases {
        let tx = register_test(task_id, "session-F", run_queue());
        tx.send(status).expect("status channel open");
        let meta = cancel_by_task(&cx(), task_id).expect("registered task is found");
        assert_eq!(meta.already_finished, finished, "{task_id}");
    }
}

#[tokio::test]
async fn cancel_all_clears_everything() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    let _a = register_test_with_thread("task-all-1", "session-A", Some("thread-1"), rq.clone());
    // Headless (no parent thread) — aborted, but contributes no thread id.
    let _b = register_test_with_thread("task-all-2", "session-B", None, rq);

    let cancelled_threads = cancel_all(&cx());
    assert!(
        cancelled_threads.contains(&"thread-1".to_string()),
        "cancel_all should report the parent thread of the cancelled sub-agent"
    );
    assert!(
        !cancelled_threads.iter().any(|t| t.is_empty()),
        "headless sub-agents must not contribute an id"
    );

    assert_eq!(
        steer(
            &cx(),
            "task-all-1",
            "session-A",
            "x".into(),
            QueueLane::Steer
        )
        .await,
        Err(SteerError::Unknown)
    );
    assert_eq!(
        steer(
            &cx(),
            "task-all-2",
            "session-B",
            "x".into(),
            QueueLane::Steer
        )
        .await,
        Err(SteerError::Unknown)
    );
    // Registry is empty now.
    assert!(cancel_all(&cx()).is_empty());
}

#[tokio::test]
async fn stop_for_thread_aborts_the_threads_running_children() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    // A real detached child that would otherwise run forever — the shape the
    // Stop button used to leave behind.
    let child = tokio::spawn(std::future::pending::<()>());
    let (_tx, rx) = status_channel();
    register(
        &cx(),
        "task-stop-1".into(),
        "researcher".into(),
        "session-stop".into(),
        None,
        None,
        test_workspace(),
        Some("thread-stop".into()),
        rq.clone(),
        child.abort_handle(),
        rx,
    )
    .expect("fixture registration admitted");
    // Another thread's child must survive the stop.
    let _other =
        register_test_with_thread("task-stop-other", "session-stop", Some("thread-keep"), rq);

    let stopped = stop_for_thread(&cx(), "thread-stop");
    assert_eq!(stopped, vec!["task-stop-1".to_string()]);

    let joined = tokio::time::timeout(Duration::from_secs(2), child)
        .await
        .expect("aborted child finishes promptly");
    assert!(
        joined.expect_err("child was aborted").is_cancelled(),
        "stop must abort the detached child task"
    );
    assert_eq!(
        steer(
            &cx(),
            "task-stop-1",
            "session-stop",
            "x".into(),
            QueueLane::Steer
        )
        .await,
        Err(SteerError::Unknown)
    );
    assert!(
        steer(
            &cx(),
            "task-stop-other",
            "session-stop",
            "x".into(),
            QueueLane::Steer
        )
        .await
        .is_ok(),
        "a different thread's sub-agent is untouched"
    );
    assert!(
        stop_for_thread(&cx(), "thread-stop").is_empty(),
        "idempotent"
    );

    prune("task-stop-other");
}
