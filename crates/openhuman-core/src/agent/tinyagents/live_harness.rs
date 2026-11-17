//! The tool harness behind a live (realtime voice) agent session.
//!
//! A live session has no model loop of its own here — the live provider holds
//! the conversation — but every tool call it makes must pass the same gates a
//! typed chat turn's calls do. This module assembles an `AgentHarness` with
//! the session's exact tool surface and the **tool-boundary** part of the turn
//! harness's middleware stack, in the same order as
//! [`install_turn_middleware`](super::turn_middleware):
//!
//! 1. the SDK tool-policy projection (`require_sandbox`),
//! 2. human-in-the-loop approval (`ApprovalSecurityMiddleware` through the
//!    global `ApprovalGate`),
//! 3. the CLI/RPC-only scope gate,
//! 4. the session's `ToolPolicyMiddleware` (deny / require-approval rules),
//! 5. credential scrubbing of every tool result,
//! 6. malformed-argument recovery,
//! 7. bare packed-tool routing and embedder tool hooks.
//!
//! Model-loop middleware (context ladder, compaction, prompt-cache guard,
//! budgets, repeat breakers, memory packs) is deliberately absent: nothing in
//! a live session calls a model through this harness.

use std::collections::HashSet;
use std::sync::Arc;

use tinyagents_harness::middleware::{
    ApprovalGateMiddleware, ArgRecoveryMiddleware, ToolPolicyMiddleware as TaToolPolicyMiddleware,
};
use tinyagents_harness::runtime::AgentHarness;

use super::host::OpenHumanRunContext;
use super::middleware;
use super::tool_registration::{capability_diagnostics, host_tool_set_builder};
use super::turn_policy::run_policy_for;
use super::ToolPolicyEnforcement;

/// Generous tool-call budget for one live conversation; the harness's
/// per-call admission still counts against it.
pub(crate) const LIVE_MAX_TOOL_CALLS: usize = 400;

/// The tools and policy a live session runs with.
pub(crate) struct LiveToolSurface {
    /// The durable tool registry, then any synthesised tools — the order turn
    /// dispatch resolves names in.
    pub(crate) tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    /// Names the model may call (the session's visible, policy-allowed set).
    pub(crate) allowed: HashSet<String>,
    /// The session's fail-closed tool policy.
    pub(crate) tool_policy: Option<ToolPolicyEnforcement>,
    /// Whether the session is bound to a chat thread (thread-scoped tools are
    /// only offered then).
    pub(crate) has_thread: bool,
    /// The session's agent/turn tool callbacks, registered after the
    /// runtime's global ones.
    pub(crate) tool_hooks:
        Vec<Arc<dyn tinyagents_hooks::tool::ToolHook<super::host::OpenHumanRunContext>>>,
    /// The context the session acts under (its embedder hooks come from it).
    pub(crate) dispatch: crate::core::runtime::DispatchCtx,
}

/// Builds the live session's tool harness.
pub(crate) fn assemble_live_tool_harness(
    surface: LiveToolSurface,
) -> AgentHarness<(), OpenHumanRunContext> {
    let LiveToolSurface {
        tool_sets,
        allowed,
        tool_policy,
        has_thread,
        tool_hooks,
        dispatch,
    } = surface;
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    harness.with_policy(run_policy_for(LIVE_MAX_TOOL_CALLS, false));
    crate::tools::timeout::install_harness_tool_timeouts(&mut harness);

    let allowed = Some(allowed);
    let report = host_tool_set_builder(&tool_sets, &allowed, has_thread, &HashSet::new())
        .register(&mut harness, None);
    let (tool_count, names) = (report.registered.len(), report.plan.candidates);
    let diagnostics = capability_diagnostics(&[], &tool_sets, &report.registered);
    for diagnostic in &diagnostics {
        tracing::warn!(
            kind = diagnostic.kind.as_str(),
            name = %diagnostic.name,
            "[voice-live] tool registry diagnostic: {}",
            diagnostic.message
        );
    }
    tracing::debug!(tool_count, tools = ?names, "[voice-live] assembled live tool harness");

    harness.push_middleware(Arc::new(
        TaToolPolicyMiddleware::new(harness.tools().policies()).require_sandbox(true),
    ));
    harness.push_tool_middleware(Arc::new(ApprovalGateMiddleware::new(
        "approval_security",
        Arc::new(middleware::approval::ApprovalSecurityMiddleware::new(
            tool_sets.clone(),
        )),
    )));
    harness.push_tool_middleware(Arc::new(
        tinyagents_harness::middleware::CliRpcOnlyMiddleware::new(tool_sets.clone()),
    ));
    let route_session = tool_policy
        .as_ref()
        .map(|enforcement| enforcement.session.clone());
    if let Some(enforcement) = tool_policy {
        harness.push_tool_middleware(Arc::new(
            middleware::tool_policy::ToolPolicyMiddleware::new(
                enforcement.policy,
                enforcement.session,
                tool_sets.clone(),
                enforcement.channel,
            ),
        ));
    }
    harness.push_tool_middleware(Arc::new(
        middleware::credential_scrub::credential_scrub_middleware(),
    ));
    harness.push_middleware(Arc::new(ArgRecoveryMiddleware::new(tool_sets.clone())));
    let registered_tools = harness.tools().names();
    harness.push_middleware(Arc::new(
        middleware::packed_tool_route::packed_tool_route_middleware(
            registered_tools,
            route_session,
        ),
    ));
    let mut embedder_tool_hooks = crate::agent::hooks::embedder_tool_hooks_in(&dispatch);
    embedder_tool_hooks.extend(tool_hooks);
    if !embedder_tool_hooks.is_empty() {
        harness.push_middleware(Arc::new(tinyagents_hooks::tool::ToolHooksMiddleware::<
            super::host::OpenHumanRunContext,
        >::new(embedder_tool_hooks)));
    }
    harness
}

#[cfg(test)]
#[path = "live_harness_tests.rs"]
mod tests;
