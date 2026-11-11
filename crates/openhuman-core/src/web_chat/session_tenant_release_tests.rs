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
    eprintln!("BELT {names:?} ctx={} ", Arc::strong_count(&profile));
    assert!(
        !profile.tenant_in_use(),
        "the session agent kept past its turn pins the tenant; belt: {names:?}"
    );
    drop(agent);
}

// TEMP-BISECT (remove before commit)
#[tokio::test]
async fn temp_bisect_tool_holders() {
    let tmp = tempfile::tempdir().unwrap();
    let config = super::session_checkout_tests::test_config(&tmp).await;
    let root = CoreContext::for_test(DomainSet::full(), None);
    let profile = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::advertised())
            .profile("p1"),
    );
    let cfg = config.clone();
    let mut tools = CoreContext::scope_with_turn_origin(Arc::clone(&profile), None, async move {
        crate::tools::ops::all_tools(
            Arc::new(cfg.clone()),
            &Arc::new(crate::security::SecurityPolicy::default()),
            crate::security::AuditSink::disabled(),
            &crate::config::BrowserConfig::default(),
            &crate::config::HttpRequestConfig::default(),
            &cfg.action_dir,
            &cfg,
        )
        .await
    })
    .await;
    eprintln!("BISECT start in_use={} n={}", profile.tenant_in_use(), tools.len());
    while let Some(tool) = tools.pop() {
        let before = profile.tenant_in_use();
        let name = tool.name().to_string();
        drop(tool);
        if before && !profile.tenant_in_use() {
            eprintln!("BISECT holder={name}");
        }
    }
    eprintln!("BISECT end in_use={}", profile.tenant_in_use());
}
