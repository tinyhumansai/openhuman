use super::*;
use crate::core::runtime::{context::CoreContext, ContextOverlay, DomainSet};
use crate::tools::toolpacks::ToolGroups;
use crate::web_chat::runner::runtime;
use crate::web_chat::track_parallel_turn_for_test;

fn agent(parent: &Arc<CoreContext>, id: &str) -> Arc<CoreContext> {
    parent.derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            ToolGroups::none(),
        )
        .session_agent(id),
    )
}

#[tokio::test]
async fn each_agent_context_owns_its_turn_tables() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let alpha = agent(&root, "alpha");
    let beta = agent(&root, "beta");

    let (alpha_sessions, alpha_runtime) = {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&alpha));
        (thread_sessions(&scope_ctx), runtime(&scope_ctx))
    };
    let (beta_sessions, beta_runtime) = {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(beta);
        (thread_sessions(&scope_ctx), runtime(&scope_ctx))
    };
    let alpha_again = runtime(&crate::core::runtime::DispatchCtx::new(alpha));

    assert!(!Arc::ptr_eq(&alpha_sessions, &beta_sessions));
    assert!(!Arc::ptr_eq(&alpha_runtime, &beta_runtime));
    assert!(Arc::ptr_eq(&alpha_runtime, &alpha_again));
}

#[tokio::test]
async fn two_profiles_on_the_same_thread_id_hold_two_keys() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let profile = |id: &str| {
        root.derive_with(
            ContextOverlay::new(
                crate::config::Config::default(),
                DomainSet::kernel(),
                ToolGroups::none(),
            )
            .profile(id)
            .session_agent(id),
        )
    };
    let (alice, bob) = (profile("u-alice"), profile("u-bob"));
    let (ctx_a, ctx_b) = (
        DispatchCtx::new(Arc::clone(&alice)),
        DispatchCtx::new(Arc::clone(&bob)),
    );
    let key_a = key_for(&ctx_a, "t1");
    let key_b = key_for(&ctx_b, "t1");
    assert_ne!(key_a, key_b);
    assert_eq!(unscope(&ctx_a, &key_a), "t1");
    assert_eq!(
        unscope(&ctx_a, &key_b),
        key_b,
        "another tenant's key is not unscoped"
    );
    // The explicit context wins over whatever scope the caller happens to run
    // in, and concurrent callers do not cross.
    let (under_bob, under_alice) = tokio::join!(
        CoreContext::scope(Arc::clone(&bob), async { key_for(&ctx_a, "t1") }),
        CoreContext::scope(Arc::clone(&alice), async { key_for(&ctx_b, "t1") }),
    );
    assert_eq!(under_bob, key_a);
    assert_eq!(under_alice, key_b);
    assert_eq!(thread_id_of_key(&key_b), Some("t1"));
    // The desktop keeps the bare id.
    assert_eq!(key_in(&crate::core::runtime::Tenant::default(), "t1"), "t1");
}

#[tokio::test]
async fn a_user_cancel_under_the_fence_placeholder_id_keeps_the_callers_client() {
    let token = tokio_util::sync::CancellationToken::new();
    track_parallel_turn_for_test(
        &crate::core::runtime::DispatchCtx::ambient(),
        "fence-thread",
        token.clone(),
    )
    .await;
    let mut events = crate::web_chat::subscribe_web_channel_events();

    crate::web_chat::ops::channel_ops::cancel_chat_scoped(
        &DispatchCtx::ambient(),
        "profile-fence",
        "fence-thread",
        None,
    )
    .await
    .unwrap();

    let seen = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(event)
                    if event.event_name() == "chat_cancelled"
                        && event.context().thread_id == "fence-thread" =>
                {
                    return event.context().client_id.clone();
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(error) => panic!("event stream closed: {error}"),
            }
        }
    })
    .await
    .expect("the cancelled turn announces itself");
    assert_eq!(seen, "profile-fence");
    tokio::time::timeout(std::time::Duration::from_secs(5), token.cancelled())
        .await
        .expect("tearing the turn down reaches its watcher");
}
