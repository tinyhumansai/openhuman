//! Background completions keep SaaS profiles apart: thread ids are only
//! unique per user, so one user's Stop or delete on `t1` must never gate,
//! drop or reach another user's `t1`.

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;

use super::background_completions::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::tools::toolpacks::ToolGroups;

fn profile(name: &str, workspace: &std::path::Path) -> Arc<CoreContext> {
    let config = crate::config::Config {
        workspace_dir: workspace.to_path_buf(),
        ..crate::config::Config::default()
    };
    CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none())
            .profile(name)
            .session_agent(name),
    )
}

async fn record(ctx: &Arc<CoreContext>, ws: &std::path::Path, task: &str) {
    record_completion(
        &DispatchCtx::new(Arc::clone(ctx)),
        ws,
        "sess-1",
        task,
        "researcher",
        "done",
        Some("t1".into()),
    )
    .await;
}

fn pending(ws: &std::path::Path) -> Vec<String> {
    pending_for(ws, "t1")
        .into_iter()
        .map(|r| r.task_id)
        .collect()
}

#[tokio::test]
async fn one_profiles_stop_and_delete_never_reach_anothers_thread() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let (ws_a, ws_b) = (TestWorkspace::new(), TestWorkspace::new());
    let alice = profile("u-alice", ws_a.path());
    let bob = profile("u-bob", ws_b.path());

    // Alice presses Stop on her `t1`.
    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&alice));

        discard_pending_for_thread(&scope_ctx, "t1");
    };

    // Bob's `t1` is untouched: his results still queue.
    record(&bob, ws_b.path(), "bob-1").await;
    assert_eq!(pending(ws_b.path()), ["bob-1"]);
    // Alice's late result is gated by her own Stop.
    record(&alice, ws_a.path(), "alice-1").await;
    assert!(pending(ws_a.path()).is_empty());

    // Bob deletes his `t1`; Alice re-engages hers and keeps working.
    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&bob));

        discard_for_thread(&scope_ctx, ws_b.path(), "t1");
    };
    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&alice));

        resume_stopped_thread(&scope_ctx, "t1");
        assert!(
            !mark_stopped_task_if_thread_stopped(&scope_ctx, ws_a.path(), "t1", "alice-2"),
            "bob's delete does not abort alice's child"
        );
    };
    record(&alice, ws_a.path(), "alice-2").await;
    assert_eq!(pending(ws_a.path()), ["alice-2"]);
    record(&bob, ws_b.path(), "bob-2").await;
    assert!(
        !pending(ws_b.path()).contains(&"bob-2".to_string()),
        "bob's deleted thread stays deleted"
    );

    // Each profile resolves `t1` to its own workspace.
    let resolved = |ctx: &Arc<CoreContext>| {
        CoreContext::sync_scope(Arc::clone(ctx), || {
            workspace_for_thread(&DispatchCtx::ambient(), "t1")
        })
    };
    assert_eq!(resolved(&alice).as_deref(), Some(ws_a.path()));
    assert_eq!(resolved(&bob).as_deref(), Some(ws_b.path()));
}

#[test]
fn a_saas_cancel_reaches_only_the_callers_workspace() {
    use super::running_subagents::{caller_workspace_in, CallerWorkspace};
    let ws = TestWorkspace::new();
    let alice = profile("u-alice", ws.path());
    assert!(matches!(
        caller_workspace_in(false, None),
        CallerWorkspace::All
    ));
    assert!(matches!(
        caller_workspace_in(true, None),
        CallerWorkspace::Nothing
    ));
    match caller_workspace_in(true, Some(&alice)) {
        CallerWorkspace::Only(dir) => assert_eq!(dir, ws.path()),
        _ => panic!("a scoped SaaS caller reaches its own workspace"),
    }
}

/// The explicit context decides, with no ambient scope anywhere: two profiles
/// recording and resolving the same thread id concurrently, across awaits and
/// from spawned tasks, never see each other's state.
#[tokio::test]
async fn the_explicit_context_keys_completions_without_any_ambient_scope() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let (ws_a, ws_b) = (TestWorkspace::new(), TestWorkspace::new());
    let alice = DispatchCtx::new(profile("x-alice", ws_a.path()));
    let bob = DispatchCtx::new(profile("x-bob", ws_b.path()));

    let record_for = |ctx: DispatchCtx, ws: std::path::PathBuf, task: &'static str| async move {
        // Yield between the steps: nothing may lean on a task-local scope.
        tokio::task::yield_now().await;
        record_completion(
            &ctx,
            &ws,
            "sess-x",
            task,
            "researcher",
            "done",
            Some("t-x".into()),
        )
        .await;
        tokio::task::yield_now().await;
        workspace_for_thread(&ctx, "t-x")
    };
    let spawned = tokio::spawn(record_for(
        alice.clone(),
        ws_a.path().to_path_buf(),
        "x-alice-1",
    ));
    let (bobs, alices) = (
        record_for(bob.clone(), ws_b.path().to_path_buf(), "x-bob-1").await,
        spawned.await.expect("alice's task ran"),
    );

    assert_eq!(alices.as_deref(), Some(ws_a.path()));
    assert_eq!(bobs.as_deref(), Some(ws_b.path()));
    assert_eq!(
        thread_for_session(&alice, "sess-x").as_deref(),
        Some("t-x"),
        "the session index is alice's own"
    );
    // A stop through one context gates only that profile's `t-x`.
    discard_pending_for_thread(&alice, "t-x");
    assert!(
        pending_for(ws_a.path(), "t-x").is_empty(),
        "alice's stop withdraws her queue"
    );
    assert_eq!(
        pending_for(ws_b.path(), "t-x")
            .into_iter()
            .map(|r| r.task_id)
            .collect::<Vec<_>>(),
        ["x-bob-1"],
        "bob's thread of the same id is untouched"
    );
    resume_stopped_thread(&alice, "t-x");
    discard_for_thread(&bob, ws_b.path(), "t-x");
}
