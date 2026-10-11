use super::*;
use crate::core::runtime::{ContextOverlay, DomainSet};
use crate::memory::scope::MemoryIdentity;

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

fn seen() -> (Option<String>, Option<String>) {
    (
        CoreContext::current().and_then(|c| c.session_agent().map(str::to_owned)),
        crate::memory::scope::current().and_then(|i| i.agent_id),
    )
}

#[tokio::test]
async fn a_bare_spawn_loses_the_scope_and_spawn_scoped_keeps_it() {
    let ctx = agent_context("u-asha");
    CoreContext::scope(ctx, async {
        crate::memory::scope::within(MemoryIdentity::agent("orchestrator"), async {
            let bare = tokio::spawn(async { seen() }).await.unwrap();
            assert_ne!(
                bare.0.as_deref(),
                Some("u-asha"),
                "a bare spawn falls back to the default context"
            );
            assert_eq!(bare.1, None, "and to no memory identity");

            let scoped = spawn_scoped(async { seen() }).await.unwrap();
            assert_eq!(scoped.0.as_deref(), Some("u-asha"));
            assert_eq!(scoped.1.as_deref(), Some("orchestrator"));
        })
        .await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_blocking_scoped_keeps_the_context() {
    let ctx = agent_context("u-ravi");
    let agent = CoreContext::scope(ctx, async {
        spawn_blocking_scoped(|| {
            CoreContext::current().and_then(|c| c.session_agent().map(str::to_owned))
        })
        .await
        .unwrap()
    })
    .await;
    assert_eq!(agent.as_deref(), Some("u-ravi"));
}

#[tokio::test]
async fn outside_any_scope_spawn_scoped_is_a_plain_spawn() {
    let (_, identity) = spawn_scoped(async { seen() }).await.unwrap();
    assert_eq!(identity, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_blocking_scoped_keeps_the_memory_identity() {
    let ctx = agent_context("u-ravi");
    let seen = CoreContext::scope(ctx, async {
        crate::memory::scope::within(MemoryIdentity::agent("planner"), async {
            spawn_blocking_scoped(seen).await.unwrap()
        })
        .await
    })
    .await;
    assert_eq!(seen.0.as_deref(), Some("u-ravi"));
    assert_eq!(seen.1.as_deref(), Some("planner"));
}

#[test]
fn scoping_a_large_task_keeps_its_wrapper_bounded() {
    // Background pollers capture large async state. Carrying that state
    // inline through both task-local wrappers multiplies the moves on the
    // spawning thread, including the CLI's ordinary process-main stack.
    let payload = [7u8; 256 * 1024];
    let task = async move {
        tokio::task::yield_now().await;
        std::hint::black_box(payload)[0]
    };
    let wrapped = scoped(task);
    assert!(
        std::mem::size_of_val(&wrapped) < 64 * 1024,
        "context propagation must not duplicate a large task inline ({} bytes)",
        std::mem::size_of_val(&wrapped),
    );
}
