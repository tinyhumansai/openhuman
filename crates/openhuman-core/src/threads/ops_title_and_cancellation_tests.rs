use super::*;
use crate::core::runtime::DispatchCtx;

#[tokio::test]
async fn thread_update_title_persists_new_title() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", workspace.path());

    let thread_id = "t-title";
    create_thread_with_title(&workspace, thread_id, "Original title").await;

    let outcome = thread_update_title(
        &crate::core::runtime::DispatchCtx::ambient(),
        crate::threads::UpdateConversationThreadTitleRequest {
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

/// The stored thread `thread_id` as it now stands.
async fn stored_thread(thread_id: &str) -> ConversationThread {
    let dir = crate::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    conversations_store::blocking::list_threads(DispatchCtx::ambient(), dir)
        .await
        .expect("list threads")
        .into_iter()
        .find(|thread| thread.id == thread_id)
        .expect("thread exists")
}

/// Title generation reads the thread, then waits seconds on the model. A rename
/// the user makes in that window must survive the generated title (#8134).
#[tokio::test]
async fn a_generated_title_does_not_overwrite_a_rename_made_while_titling() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "t-rename-while-titling";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;
    let observed = stored_thread(thread_id).await;

    thread_update_title(
        &DispatchCtx::ambient(),
        crate::threads::UpdateConversationThreadTitleRequest {
            thread_id: thread_id.to_string(),
            title: "Bug Test 2026".to_string(),
        },
    )
    .await
    .expect("rename");

    let returned = crate::threads::ops::support::update_title_unless_renamed(
        &DispatchCtx::ambient(),
        workspace_dir_for_test().await,
        observed,
        "Summarized notes".to_string(),
        "check my notes",
    )
    .await
    .expect("title write");
    assert_eq!(
        returned.title, "Bug Test 2026",
        "the caller sees the rename"
    );
    assert_eq!(
        stored_thread(thread_id).await.title,
        "Bug Test 2026",
        "the generated title is not stored over the rename"
    );
}

#[tokio::test]
async fn a_generated_title_replaces_an_untouched_placeholder() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "t-untouched-placeholder";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;
    let observed = stored_thread(thread_id).await;

    let returned = crate::threads::ops::support::update_title_unless_renamed(
        &DispatchCtx::ambient(),
        workspace_dir_for_test().await,
        observed,
        "Summarized notes".to_string(),
        "check my notes",
    )
    .await
    .expect("title write");
    assert_eq!(returned.title, "Summarized notes");
    assert_eq!(stored_thread(thread_id).await.title, "Summarized notes");
}

/// Two titling calls can both read the placeholder; the interim title the
/// first one writes is not a rename and the second still replaces it.
#[tokio::test]
async fn a_generated_title_replaces_an_interim_title_written_meanwhile() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", workspace.path());
    let thread_id = "t-interim-meanwhile";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;
    let observed = stored_thread(thread_id).await;
    let interim = tinyagents_harness::title::title_from_user_message("check my notes")
        .expect("interim title");

    thread_update_title(
        &DispatchCtx::ambient(),
        crate::threads::UpdateConversationThreadTitleRequest {
            thread_id: thread_id.to_string(),
            title: interim,
        },
    )
    .await
    .expect("interim write");

    let returned = crate::threads::ops::support::update_title_unless_renamed(
        &DispatchCtx::ambient(),
        workspace_dir_for_test().await,
        observed,
        "Summarized notes".to_string(),
        "check my notes",
    )
    .await
    .expect("title write");
    assert_eq!(returned.title, "Summarized notes");
    assert_eq!(stored_thread(thread_id).await.title, "Summarized notes");
}

async fn workspace_dir_for_test() -> std::path::PathBuf {
    crate::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir
}

#[tokio::test]
async fn thread_update_title_returns_error_for_missing_thread() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", workspace.path());

    let err = thread_update_title(
        &crate::core::runtime::DispatchCtx::ambient(),
        crate::threads::UpdateConversationThreadTitleRequest {
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

    let dispatch = crate::core::runtime::DispatchCtx::unbound();
    let mut call = Box::pin(run_to_completion(
        &dispatch,
        "test_destructive_op",
        async move {
            // Stands in for the blocking store mutation still in progress.
            let _ = parked.await;
            // Stands in for the cleanup that must never be skipped.
            flag.store(true, Ordering::SeqCst);
            Ok::<(), String>(())
        },
    ));

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

/// The detached task runs under the context the caller handed over, not the
/// one its own task happens to be scoped to, and two callers for different
/// contexts do not see each other's.
#[tokio::test]
async fn run_to_completion_runs_under_the_explicit_context() {
    use crate::core::runtime::context::CoreContext;
    use crate::core::runtime::{DispatchCtx, DomainSet};
    use std::sync::Arc;

    let a = CoreContext::for_test(DomainSet::full(), Some(std::path::PathBuf::from("/tmp/a")));
    let b = CoreContext::for_test(DomainSet::full(), Some(std::path::PathBuf::from("/tmp/b")));
    let seen_by = |dispatch: DispatchCtx| async move {
        run_to_completion(&dispatch, "test_context", async {
            tokio::task::yield_now().await;
            Ok::<_, String>(CoreContext::current())
        })
        .await
        .expect("task joins")
        .expect("the task has a context")
    };
    // `b` is the caller's own scope; `a` is handed over explicitly and wins.
    let (seen_a, seen_b) = CoreContext::scope(Arc::clone(&b), async {
        tokio::join!(
            seen_by(DispatchCtx::new(Arc::clone(&a))),
            seen_by(DispatchCtx::new(Arc::clone(&b)))
        )
    })
    .await;
    assert!(Arc::ptr_eq(&seen_a, &a));
    assert!(Arc::ptr_eq(&seen_b, &b));
}
