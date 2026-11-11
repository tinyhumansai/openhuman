//! A web-chat session agent is cached in its tenant's own state slots
//! (`web_chat::ops::thread_sessions`) once its turn ends, so anything in it
//! that holds the tenant's context strongly keeps the tenant in use
//! (`CoreContext::tenant_in_use`) and its SaaS profile slot never frees.

use std::sync::Arc;

use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
use crate::tools::toolpacks::ToolGroups;

#[tokio::test]
async fn a_cached_session_agent_does_not_keep_its_tenant_in_use() {
    let tmp = tempfile::tempdir().unwrap();
    let config = super::session_checkout_tests::test_config(&tmp).await;
    let root = CoreContext::for_test(DomainSet::full(), None);
    // Every domain and every tool group, so the whole belt is built.
    let profile = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::advertised())
            .profile("p1"),
    );
    let request = Arc::clone(&profile);
    let agent = CoreContext::scope_with_turn_origin(Arc::clone(&profile), None, async move {
        super::build_session_agent(
            &DispatchCtx::new(request),
            &config,
            "client-1",
            "thread-1",
            "orchestrator",
            None,
            None,
        )
        .await
    })
    .await
    .unwrap_or_else(|error| panic!("the session agent builds: {error}"));
    let names: Vec<&str> = agent.tools().iter().map(|tool| tool.name()).collect();
    for expected in ["create_artifact", "http_request", "web_fetch", "curl"] {
        assert!(
            names.contains(&expected),
            "{expected} is in the belt: {names:?}"
        );
    }
    // Building the session starts short background work of its own (the
    // flow-authoring prewarm) that holds the tenant while it runs; that has to
    // finish. What may not happen is a hold that outlives it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while profile.tenant_in_use() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        !profile.tenant_in_use(),
        "the session agent kept past its turn pins the tenant; belt: {names:?}"
    );
    drop(agent);
}
