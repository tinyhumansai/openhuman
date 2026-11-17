//! Releasing the completion routers (`release_all`) when the core server stops.

use super::*;
use crate::core::runtime::DispatchCtx;

/// The context these tests dispatch under: the calling task's own.
fn cx() -> DispatchCtx {
    DispatchCtx::ambient()
}

fn workspace() -> TestWorkspace {
    TestWorkspace::new()
}

fn pending_ids(ws: &Path, thread: &str) -> Vec<String> {
    pending_for(ws, thread)
        .into_iter()
        .map(|r| r.task_id)
        .collect()
}

#[tokio::test]
async fn release_all_closes_every_router_and_the_log_stays_replayable() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    record_completion(
        &cx(),
        w,
        "sess-rel",
        "sub-1",
        "researcher",
        "kept on disk",
        Some("thread-rel".into()),
    )
    .await;

    assert!(release_all().await >= 1);
    assert!(
        !state().stores.contains_key(&super::router_scope(w)),
        "the router (and its log handle) is dropped"
    );
    assert!(
        workspace_for_thread(&cx(), "thread-rel").is_none(),
        "process-local maps are cleared"
    );

    // The next boot finds the undelivered completion from the log alone.
    assert_eq!(recover_pending_threads(&cx(), w), ["thread-rel"]);
}

#[tokio::test(start_paused = true)]
async fn release_all_waits_for_a_router_still_in_use() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let in_use = router_for_workspace(w);
    let releasing = tokio::spawn(release_all());
    // Well inside the drain window: still waiting on the held router.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert!(
        !releasing.is_finished(),
        "waits while a delivery holds the router"
    );
    drop(in_use);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(releasing.await.unwrap() >= 1);
}

#[tokio::test(start_paused = true)]
async fn release_all_gives_up_waiting_after_the_drain_window() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    let _held_forever = router_for_workspace(w);
    let released = release_all().await;
    assert!(
        released >= 1,
        "stragglers are dropped once the window passes"
    );
    assert!(!state().stores.contains_key(&super::router_scope(w)));
}

#[tokio::test]
async fn release_all_keeps_the_stop_and_delete_gates() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = workspace();
    let w = ws.path();
    discard_pending_for_thread(&cx(), "thread-keep-stop");
    state().deleted_threads.insert("thread-keep-del".into());
    release_all().await;

    // A detached child finishing after the release is still refused.
    record_completion(
        &cx(),
        w,
        "s",
        "sub-1",
        "r",
        "x",
        Some("thread-keep-stop".into()),
    )
    .await;
    record_completion(
        &cx(),
        w,
        "s",
        "sub-2",
        "r",
        "x",
        Some("thread-keep-del".into()),
    )
    .await;
    assert!(pending_ids(w, "thread-keep-stop").is_empty());
    assert!(pending_ids(w, "thread-keep-del").is_empty());
    resume_stopped_thread(&cx(), "thread-keep-stop");
    state().deleted_threads.remove("thread-keep-del");
}
