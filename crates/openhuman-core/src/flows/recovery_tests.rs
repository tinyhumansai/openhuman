use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn live_agent(agent: &str) -> Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{AgentContextRegistry, ContextOverlay, CoreContext, DomainSet};
    let context = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(Config::default(), DomainSet::full(), Default::default())
            .session_agent(agent),
    );
    AgentContextRegistry::register(agent, &context);
    context
}

/// On a shared backend the boot sweep registers no scope's storage: the
/// registration alone prepares collections, once per agent.
#[tokio::test]
async fn a_shared_backend_boot_sweep_visits_no_scope() {
    let _guard = crate::storage::test_support::install_backend_for_test(
        crate::storage::test_support::memory_backend(),
    );
    let agents = ["flows-sweep-a", "flows-sweep-b", "flows-sweep-c"];
    let contexts: Vec<_> = agents.iter().map(|agent| live_agent(agent)).collect();
    let visits = AtomicUsize::new(0);
    let ctx = DispatchCtx::host_default();

    let count = |_: DispatchCtx| async {
        visits.fetch_add(1, Ordering::SeqCst);
        1
    };
    assert_eq!(sweep_scopes(&ctx, true, count).await, 0);
    assert_eq!(visits.load(Ordering::SeqCst), 0);

    let swept = sweep_scopes(&ctx, false, count).await;
    assert!(visits.load(Ordering::SeqCst) >= agents.len());
    assert_eq!(swept, visits.load(Ordering::SeqCst));

    for (agent, context) in agents.iter().zip(&contexts) {
        crate::core::runtime::AgentContextRegistry::deregister(agent, context);
    }
}
