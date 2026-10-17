use super::*;
use crate::core::runtime::{AgentContextRegistry, ContextOverlay, DomainSet};
use crate::tools::toolpacks::ToolGroups;

fn agent_context(parent: &Arc<CoreContext>, id: &str, workspace: &str) -> Arc<CoreContext> {
    let mut config = crate::config::Config::default();
    config.workspace_dir = std::path::PathBuf::from(workspace);
    parent.derive_with(
        ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none()).session_agent(id),
    )
}

fn tenant_of(agent: &str) -> Tenant {
    Tenant {
        profile: None,
        agent: Some(agent.to_string()),
    }
}

#[tokio::test]
async fn an_event_is_handled_under_its_tenant_while_another_is_active() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let a = agent_context(&root, "evt-ctx-a", "/tmp/evt-ctx-a");
    let b = agent_context(&root, "evt-ctx-b", "/tmp/evt-ctx-b");
    AgentContextRegistry::register("evt-ctx-a", &a);
    AgentContextRegistry::register("evt-ctx-b", &b);

    // Published while B is the active context: the event still names A.
    let event_tenant = DispatchCtx::new(Arc::clone(&a)).event_tenant();
    assert_eq!(event_tenant, Some(tenant_of("evt-ctx-a")));

    let (seen_a, seen_b) = CoreContext::scope(Arc::clone(&b), async {
        let for_a = DispatchCtx::for_event(event_tenant.as_ref()).expect("a is live");
        assert!(Arc::ptr_eq(for_a.core().unwrap(), &a));
        let config_a = for_a.load_config().await.unwrap();
        // Across an await and into a spawned task the event's tenant holds.
        let spawned = tokio::spawn(async move { for_a.load_config().await.unwrap() })
            .await
            .unwrap();
        assert_eq!(config_a.workspace_dir, spawned.workspace_dir);

        let for_b = DispatchCtx::for_event(Some(&tenant_of("evt-ctx-b"))).expect("b is live");
        (
            config_a.workspace_dir,
            for_b.load_config().await.unwrap().workspace_dir,
        )
    })
    .await;

    assert_eq!(seen_a, std::path::PathBuf::from("/tmp/evt-ctx-a"));
    assert_eq!(seen_b, std::path::PathBuf::from("/tmp/evt-ctx-b"));
    AgentContextRegistry::deregister("evt-ctx-a", &a);
    AgentContextRegistry::deregister("evt-ctx-b", &b);
}

#[tokio::test]
async fn concurrent_events_for_two_tenants_do_not_cross() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let a = agent_context(&root, "evt-conc-a", "/tmp/evt-conc-a");
    let b = agent_context(&root, "evt-conc-b", "/tmp/evt-conc-b");
    AgentContextRegistry::register("evt-conc-a", &a);
    AgentContextRegistry::register("evt-conc-b", &b);

    let handle = |tenant: Tenant| async move {
        let ctx = DispatchCtx::for_event(Some(&tenant)).expect("live tenant");
        tokio::task::yield_now().await;
        ctx.load_config().await.unwrap().workspace_dir
    };
    let (seen_a, seen_b) = tokio::join!(
        tokio::spawn(handle(tenant_of("evt-conc-a"))),
        tokio::spawn(handle(tenant_of("evt-conc-b"))),
    );
    assert_eq!(seen_a.unwrap(), std::path::PathBuf::from("/tmp/evt-conc-a"));
    assert_eq!(seen_b.unwrap(), std::path::PathBuf::from("/tmp/evt-conc-b"));
    AgentContextRegistry::deregister("evt-conc-a", &a);
    AgentContextRegistry::deregister("evt-conc-b", &b);
}

#[test]
fn an_event_whose_tenant_is_gone_is_skipped_not_run_as_the_default() {
    let gone = tenant_of("evt-ctx-never-registered");
    assert!(DispatchCtx::for_event(Some(&gone)).is_none());
    let missing_profile = Tenant {
        profile: Some("no-such-profile".into()),
        agent: None,
    };
    assert!(DispatchCtx::for_event(Some(&missing_profile)).is_none());
}

#[test]
fn an_untagged_event_is_the_process_defaults_outside_saas() {
    if crate::core::runtime::is_saas() {
        assert!(DispatchCtx::for_event(None).is_none());
    } else {
        assert!(DispatchCtx::for_event(None).is_some());
        // The host's own tenant is untagged too.
        assert!(DispatchCtx::for_event(Some(&Tenant::default())).is_some());
    }
}
