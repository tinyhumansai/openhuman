//! Dispatch of a registered controller: the gate, then the handler.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Map, Value};

use super::{internal_registry, registry_view, visible, ControllerFuture};
use crate::core::runtime::{CoreContext, DispatchCtx};

type Dispatched = Pin<Box<dyn Future<Output = Option<Result<Value, String>>> + Send + 'static>>;

/// Attempts to invoke a registered RPC method by name.
///
/// Checks both the agent-facing controller registry and the internal-only registry,
/// so internal-only write paths (e.g. the MCP write-audit list) are routable
/// even though they are not included in agent tool listings.
///
/// The handler receives `ctx` as its first argument. A call that arrives with no
/// bound context inherits the calling task's scope, or the process default.
/// Until every reader below the handler has moved off the ambient context
/// (#7801), the resolved context is also scoped onto the task for the whole
/// call (registry view, domain gate, handler), so those readers resolve to the
/// context the caller named rather than to whatever the task carried.
///
/// Returns `None` if the method is not found in either registry.
pub async fn try_invoke_registered_rpc(
    ctx: DispatchCtx,
    method: &str,
    params: Map<String, Value>,
) -> Option<Result<Value, String>> {
    let core = ctx.into_core().or_else(CoreContext::current);
    let ctx = match &core {
        Some(core) => DispatchCtx::new(core.clone()),
        None => DispatchCtx::unbound(),
    };
    // Boxed so the concrete `TaskLocalFuture` type does not escape into this
    // fn's future: without the type erasure the `Send` auto-trait solver
    // overflows on the largest handler futures (E0275).
    let call: Dispatched = Box::pin(invoke_registered(ctx, method.to_owned(), params));
    match core {
        Some(core) => CoreContext::scope(core, call).await,
        None => call.await,
    }
}

async fn invoke_registered(
    ctx: DispatchCtx,
    method: String,
    params: Map<String, Value>,
) -> Option<Result<Value, String>> {
    let view = registry_view();
    let grouped = view
        .iter()
        .chain(internal_registry().iter())
        .find(|g| g.controller.rpc_method_name() == method)?;

    // DomainSet gate (#4796): a method whose group is disabled under the call's
    // context reports as an unknown method (`None`) — the same result a caller
    // gets for a genuinely-unregistered method, so a gated domain's controllers
    // are indistinguishable from absent. Enforced HERE (dispatch), not in
    // schema/validation lookups, to avoid a validate/dispatch split.
    if !visible(grouped) {
        log::debug!(
            "[rpc][domain-gate] method '{method}' suppressed — group {:?} disabled or method \
             outside the user surface in the active scope",
            grouped.group
        );
        return None;
    }

    let handler = grouped.controller.handler;
    let fut: ControllerFuture = handler(ctx, params);
    Some(fut.await)
}
