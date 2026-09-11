use super::*;

#[tokio::test]
async fn thread_update_title_persists_new_title() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());

    let thread_id = "t-title";
    create_thread_with_title(&workspace, thread_id, "Original title").await;

    let outcome = thread_update_title(
        crate::openhuman::memory::UpdateConversationThreadTitleRequest {
            thread_id: thread_id.to_string(),
            title: "  Invoice follow-up  ".to_string(),
        },
    )
    .await
    .expect("thread_update_title");

    let summary = outcome.value.data.expect("data envelope");
    assert_eq!(
        summary.title, "Invoice follow-up",
        "title must be trimmed and persisted"
    );
    assert_eq!(summary.id, thread_id);
}

#[tokio::test]
async fn thread_update_title_returns_error_for_missing_thread() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", workspace.path());

    let err = thread_update_title(
        crate::openhuman::memory::UpdateConversationThreadTitleRequest {
            thread_id: "nonexistent-thread".to_string(),
            title: "New title".to_string(),
        },
    )
    .await
    .expect_err("missing thread must return an error");

    assert!(
        err.contains("update title"),
        "error must describe the update-title failure, got: {err}"
    );
}

/// Review follow-up on #5282: moving the conversation store onto the blocking
/// pool put an `.await` between a destructive store mutation and the cleanup
/// that has to follow it (web-session invalidation, sub-agent cancellation,
/// turn-snapshot deletion).
///
/// `spawn_blocking` work is never cancelled by dropping its `JoinHandle`, so a
/// caller that goes away in that window — client disconnect, RPC timeout —
/// used to leave the thread deleted from the store while every one of those
/// cleanup steps was skipped. `run_to_completion` owns the mutation *and* the
/// cleanup in one spawned task, so the tail runs regardless of the caller.
#[tokio::test]
async fn run_to_completion_runs_the_tail_after_the_caller_is_dropped() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let tail_ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&tail_ran);
    let (release, parked) = tokio::sync::oneshot::channel::<()>();

    let mut call = Box::pin(run_to_completion("test_destructive_op", async move {
        // Stands in for the blocking store mutation still in progress.
        let _ = parked.await;
        // Stands in for the cleanup that must never be skipped.
        flag.store(true, Ordering::SeqCst);
        Ok::<(), String>(())
    }));

    // Poll once so the inner task is spawned, then abandon the caller — exactly
    // what dropping the RPC future does.
    tokio::select! {
        biased;
        _ = &mut call => panic!("the inner task should still be parked"),
        _ = tokio::task::yield_now() => {}
    }
    drop(call);

    assert!(
        !tail_ran.load(Ordering::SeqCst),
        "the tail must not have run before the mutation completed"
    );

    // The store mutation finishes after the caller is already gone.
    release.send(()).expect("release the parked task");

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !tail_ran.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cleanup must still run after the caller's future was dropped");
}
