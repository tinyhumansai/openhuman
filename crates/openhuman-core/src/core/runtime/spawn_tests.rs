use super::*;
use crate::core::runtime::{ContextOverlay, DispatchCtx, DomainSet};

fn agent_context(agent: &str) -> std::sync::Arc<CoreContext> {
    CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .session_agent(agent),
    )
}

fn seen() -> Option<String> {
    CoreContext::current().and_then(|c| c.session_agent().map(str::to_owned))
}

#[tokio::test]
async fn a_bare_spawn_loses_the_scope_and_a_context_spawn_keeps_it() {
    let ctx = agent_context("u-asha");
    let dispatch = DispatchCtx::new(ctx.clone());
    // Spawned from outside any scope: the task acts for the context it was
    // handed, not for whatever its spawner happened to run in.
    let handed = dispatch.spawn(async { seen() }).await.unwrap();
    assert_eq!(handed.as_deref(), Some("u-asha"));

    CoreContext::scope(ctx, async {
        let bare = tokio::spawn(async { seen() }).await.unwrap();
        assert_ne!(
            bare.as_deref(),
            Some("u-asha"),
            "a bare spawn falls back to the default context"
        );
    })
    .await;
}

#[tokio::test]
async fn two_contexts_spawn_tasks_that_do_not_cross() {
    let a = DispatchCtx::new(agent_context("u-asha"));
    let b = DispatchCtx::new(agent_context("u-ravi"));
    let (ra, rb) = tokio::join!(
        a.spawn(async {
            tokio::task::yield_now().await;
            seen()
        }),
        b.spawn(async {
            tokio::task::yield_now().await;
            seen()
        })
    );
    assert_eq!(ra.unwrap().as_deref(), Some("u-asha"));
    assert_eq!(rb.unwrap().as_deref(), Some("u-ravi"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_blocking_keeps_the_handed_context() {
    let dispatch = DispatchCtx::new(agent_context("u-ravi"));
    let agent = dispatch
        .spawn_blocking(|| {
            CoreContext::current().and_then(|c| c.session_agent().map(str::to_owned))
        })
        .await
        .unwrap();
    assert_eq!(agent.as_deref(), Some("u-ravi"));
}

#[tokio::test]
async fn a_task_spawned_from_a_context_keeps_it_across_awaits_and_nested_spawns() {
    let dispatch = DispatchCtx::new(agent_context("u-asha"));
    let seen_in_child = dispatch
        .spawn(async {
            tokio::task::yield_now().await;
            // A nested spawn from inside re-hands the ambient context.
            DispatchCtx::ambient()
                .spawn(async { seen() })
                .await
                .unwrap()
        })
        .await
        .unwrap();
    assert_eq!(seen_in_child.as_deref(), Some("u-asha"));
}

#[test]
fn entering_a_large_task_keeps_its_wrapper_bounded() {
    // Background pollers capture large async state. Carrying that state
    // inline through both task-local wrappers multiplies the moves on the
    // spawning thread, including the CLI's ordinary process-main stack.
    let payload = [7u8; 256 * 1024];
    let task = async move {
        tokio::task::yield_now().await;
        std::hint::black_box(payload)[0]
    };
    let wrapped = DispatchCtx::new(agent_context("u-asha")).enter(task);
    assert!(
        std::mem::size_of_val(&wrapped) < 64 * 1024,
        "context propagation must not duplicate a large task inline ({} bytes)",
        std::mem::size_of_val(&wrapped),
    );
}

#[tokio::test]
async fn a_task_acts_for_the_context_it_was_handed_while_another_is_active() {
    let handed = DispatchCtx::new(agent_context("u-asha"));
    // The spawner runs inside a different tenant's scope; the task must act
    // for the one it was handed, async and blocking alike.
    let (task, blocking) = CoreContext::scope(agent_context("u-ravi"), async {
        let task = handed.spawn(async { seen() }).await.unwrap();
        let blocking = handed.spawn_blocking(seen).await.unwrap();
        (task, blocking)
    })
    .await;
    assert_eq!(task.as_deref(), Some("u-asha"));
    assert_eq!(blocking.as_deref(), Some("u-asha"));
}
