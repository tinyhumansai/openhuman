use super::*;

#[test]
fn host_agent_context_takes_precedence_over_flow_dispatch_context() {
    let dispatch_context =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::none(), None);
    let host_context =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::none(), None);
    let dispatch = crate::core::runtime::DispatchCtx::new(dispatch_context);

    let selected = host_core_context(&dispatch, Some(&host_context))
        .and_then(|context| context.upgrade())
        .expect("host context remains available");
    assert!(std::sync::Arc::ptr_eq(&selected, &host_context));
}

#[test]
fn an_unhosted_harness_uses_the_flow_dispatch_context() {
    let dispatch_context =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::none(), None);
    let dispatch = crate::core::runtime::DispatchCtx::new(dispatch_context.clone());

    let selected = host_core_context(&dispatch, None)
        .and_then(|context| context.upgrade())
        .expect("flow dispatch context remains available");
    assert!(std::sync::Arc::ptr_eq(&selected, &dispatch_context));
}
