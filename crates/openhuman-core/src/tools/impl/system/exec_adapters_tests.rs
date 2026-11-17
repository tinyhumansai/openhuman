use super::*;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
use crate::tools::toolpacks::ToolGroups;
use std::collections::BTreeMap;

fn tool_context_for_profile(
    profile: &str,
) -> (
    Arc<CoreContext>,
    tinyagents_harness::tool::ToolExecutionContext,
) {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let core = root.derive_with(
        ContextOverlay::new(Config::default(), DomainSet::full(), ToolGroups::none())
            .profile(profile),
    );
    let run = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .into_tinyagents(tinyagents_harness::context::RunConfig::new(
            "install-audit-test",
        ));
    let context = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new(profile),
    );
    (core, context)
}

#[tokio::test]
async fn concurrent_install_calls_keep_their_own_audit_tenant() {
    let gate = InstallGate {
        security: Arc::new(SecurityPolicy::default()),
        audit: AuditSink::disabled(),
        config: Arc::new(Config::default()),
        audit_context: DispatchCtx::unbound(),
        environment: BTreeMap::new(),
    };
    let (core_a, context_a) = tool_context_for_profile("tenant-a");
    let (core_b, context_b) = tool_context_for_profile("tenant-b");

    // Each tool invocation gets its own gate before either concurrent call
    // reaches ExecGate::record.
    let gate_a = gate.for_tool_context(Some(&context_a));
    let gate_b = gate.for_tool_context(Some(&context_b));
    let read_tenant = |gate: InstallGate| async move {
        tokio::task::yield_now().await;
        gate.audit_context.tenant().unwrap()
    };
    let (tenant_a, tenant_b) = tokio::join!(read_tenant(gate_a), read_tenant(gate_b));

    assert_eq!(tenant_a, crate::core::runtime::Tenant::of(&core_a));
    assert_eq!(tenant_b, crate::core::runtime::Tenant::of(&core_b));
}
