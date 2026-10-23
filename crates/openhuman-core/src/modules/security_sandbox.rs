//! Host client for the module's sandbox planning: `RegisterSandboxScope2`,
//! `RetireSandboxScope2`, `Plan2` and `ConsumeSandboxPlan2`.
//!
//! The module decides what may run; this file is only transport. Every call is
//! made on the attested module proxy with the request carried confidentially
//! (host paths, argv digests). A refusal, a malformed answer or an unreachable
//! module is an `Err`, and the caller must not run the command: nothing here falls
//! back to a host-side decision. Like the approval engine, the module answering
//! that it refused one request does not fault the shared client.

use tinysecurity_bus::{
    names, ConsumeSandboxPlanRequest2, ConsumeSandboxPlanResponse2, PolicyInfo,
    RegisterSandboxScopeRequest2, RetireSandboxScopeRequest2, SandboxPlanRequest2,
    SandboxPlanResponse2, SandboxScopeReceipt2,
};

use crate::config::Config;

/// The attested module's proxy for sandbox planning, loading the module first.
pub(crate) async fn production_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
    super::approval_proxy(config).await
}

/// The module's current policy generation, which every plan and its audit carry.
pub(crate) async fn generation(proxy: &tinybus::Proxy) -> Result<u64, String> {
    let info: PolicyInfo = proxy
        .call(names::methods::POLICY_INFO, ())
        .await
        .map_err(|_| "TinySecurity PolicyInfo failed")?;
    if info.generation == 0 {
        return Err("TinySecurity reported no policy generation".into());
    }
    Ok(info.generation)
}

/// Register one subject's sandbox scope.
pub(crate) async fn register_scope(
    proxy: &tinybus::Proxy,
    request: RegisterSandboxScopeRequest2,
) -> Result<SandboxScopeReceipt2, String> {
    let scope_id = request.scope.scope_id.clone();
    let receipt: SandboxScopeReceipt2 = proxy
        .call_confidential(names::methods::REGISTER_SANDBOX_SCOPE2, (request,))
        .await
        .map_err(|error| {
            log::warn!("[security][sandbox] RegisterSandboxScope2 refused: {error}");
            "TinySecurity RegisterSandboxScope2 failed".to_owned()
        })?;
    if receipt.scope_id != scope_id {
        return Err("TinySecurity returned a receipt for another sandbox scope".into());
    }
    Ok(receipt)
}

/// Retire one runtime sandbox scope and everything it issued.
pub(crate) async fn retire_scope(proxy: &tinybus::Proxy, scope_id: &str) -> Result<(), String> {
    let receipt: SandboxScopeReceipt2 = proxy
        .call_confidential(
            names::methods::RETIRE_SANDBOX_SCOPE2,
            (RetireSandboxScopeRequest2 {
                scope_id: scope_id.to_owned(),
            },),
        )
        .await
        .map_err(|_| "TinySecurity RetireSandboxScope2 failed")?;
    if receipt.scope_id != scope_id {
        return Err("TinySecurity returned a receipt for another sandbox scope".into());
    }
    Ok(())
}

/// Ask the module to plan one concrete spawn.
pub(crate) async fn plan(
    proxy: &tinybus::Proxy,
    request: SandboxPlanRequest2,
) -> Result<SandboxPlanResponse2, String> {
    proxy
        .call_confidential(names::methods::PLAN2, (request,))
        .await
        .map_err(|_| "TinySecurity Plan2 failed".to_owned())
}

/// Reserve an issued plan once, immediately before launching it.
pub(crate) async fn consume(
    proxy: &tinybus::Proxy,
    request: ConsumeSandboxPlanRequest2,
) -> Result<ConsumeSandboxPlanResponse2, String> {
    proxy
        .call_confidential(names::methods::CONSUME_SANDBOX_PLAN2, (request,))
        .await
        .map_err(|_| "TinySecurity ConsumeSandboxPlan2 failed".to_owned())
}
