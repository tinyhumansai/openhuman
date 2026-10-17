use super::*;
use crate::core::runtime::DomainSet;

#[test]
fn unbound_context_has_no_core() {
    let ctx = DispatchCtx::unbound();
    assert!(ctx.core().is_none());
    assert!(ctx.into_core().is_none());
}

#[test]
fn new_context_carries_exactly_the_given_core() {
    let core = CoreContext::for_test(DomainSet::full(), None);
    let ctx = DispatchCtx::new(Arc::clone(&core));
    assert!(Arc::ptr_eq(ctx.core().expect("bound"), &core));
    let cloned = ctx.clone();
    assert!(Arc::ptr_eq(cloned.core().expect("bound"), &core));
}

#[test]
fn from_arc_binds_the_context() {
    let core = CoreContext::for_test(DomainSet::full(), None);
    let ctx: DispatchCtx = Arc::clone(&core).into();
    assert!(Arc::ptr_eq(ctx.core().expect("bound"), &core));
}

#[tokio::test]
async fn ambient_reads_the_scoped_context_and_explicit_ignores_it() {
    let scoped = CoreContext::for_test(DomainSet::full(), None);
    let other = CoreContext::for_test(DomainSet::full(), None);
    CoreContext::scope(Arc::clone(&scoped), async {
        // An explicit context is the one handed in, whatever the task scope says.
        let explicit = DispatchCtx::new(Arc::clone(&other));
        assert!(Arc::ptr_eq(explicit.core().unwrap(), &other));
        let ambient = DispatchCtx::ambient();
        assert!(Arc::ptr_eq(ambient.core().unwrap(), &scoped));
    })
    .await;
}

fn web_origin(thread: &str) -> crate::agent::turn_origin::AgentTurnOrigin {
    crate::agent::turn_origin::AgentTurnOrigin::WebChat {
        thread_id: thread.into(),
        client_id: "client".into(),
        request_id: format!("req-{thread}").into(),
    }
}

#[tokio::test]
async fn explicit_turn_origin_wins_over_the_scoped_one_and_does_not_cross() {
    let base = CoreContext::for_test(DomainSet::full(), None);
    let a = Arc::new(base.derive_channel_turn(None, Some(web_origin("a"))));
    let b = Arc::new(base.derive_channel_turn(None, Some(web_origin("b"))));
    let thread_of = |ctx: &DispatchCtx| match ctx.turn_origin() {
        Some(crate::agent::turn_origin::AgentTurnOrigin::WebChat { thread_id, .. }) => thread_id,
        other => panic!("expected a web origin, got {other:?}"),
    };
    // Scoped to `a`, a context bound to `b` still answers `b`, across awaits
    // and from a spawned task that carries no scope at all.
    CoreContext::scope(Arc::clone(&a), async {
        let explicit = DispatchCtx::new(Arc::clone(&b));
        tokio::task::yield_now().await;
        assert_eq!(thread_of(&explicit), "b");
        assert_eq!(explicit.turn_origin_or_scoped().map(|_| ()), Some(()));
        let moved = explicit.clone();
        let from_task = tokio::spawn(async move { thread_of(&moved) })
            .await
            .unwrap();
        assert_eq!(from_task, "b");
        assert_eq!(thread_of(&DispatchCtx::new(Arc::clone(&a))), "a");
    })
    .await;
}

#[test]
fn unbound_context_carries_no_turn_origin_or_agent_state() {
    let ctx = DispatchCtx::unbound();
    assert!(ctx.turn_origin().is_none());
    assert!(ctx.host_overrides().is_none());
    assert!(ctx.agent_policy().is_none());
    assert!(!ctx.approvals_disabled());
}

#[tokio::test]
async fn host_default_is_the_process_default_whatever_the_task_scope_says() {
    // A test context is never the process default, so the scoped one below
    // must not be what `host_default` answers with.
    let scoped = CoreContext::for_test(DomainSet::full(), None);
    let default = CoreContext::default_context();
    let same = |ctx: &DispatchCtx| match (ctx.core(), &default) {
        (Some(got), Some(want)) => Arc::ptr_eq(got, want),
        (None, None) => true,
        _ => false,
    };

    let host = CoreContext::scope(Arc::clone(&scoped), async {
        // Across an await and inside a spawned task the answer is the same.
        tokio::task::yield_now().await;
        let spawned = tokio::spawn(async { DispatchCtx::host_default() })
            .await
            .unwrap();
        assert!(same(&spawned));
        DispatchCtx::host_default()
    })
    .await;
    assert!(same(&host));
    assert!(host.core().is_none_or(|got| !Arc::ptr_eq(got, &scoped)));
}

#[tokio::test]
async fn host_default_in_saas_is_unbound_even_inside_a_tenants_scope() {
    let tenant = CoreContext::for_test(DomainSet::full(), None);
    CoreContext::scope(Arc::clone(&tenant), async {
        tokio::task::yield_now().await;
        // The task's scope is a tenant's; work that arrives without its own
        // tenant must not inherit it, and must not become the operator.
        assert!(DispatchCtx::host_default_in(true).core().is_none());
        let spawned = tokio::spawn(async { DispatchCtx::host_default_in(true) })
            .await
            .unwrap();
        assert!(spawned.core().is_none());
    })
    .await;
}
