//! Host-owned selection and model routing for TinyFlows agent nodes.

use crate::inference::provider::factory::PROVIDER_OPENHUMAN;

/// Which execution path an `agent_ref` routes to (see [`crate::flows::caps_host::agent::OpenHumanAgentRunner`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentRoute {
    /// A host-registered agent (`agent::host_agents`) — run the full agent
    /// tool loop as that agent, with its host tools, in its context.
    HostAgent,
    /// A harness `AgentDefinition` exists — run the full agent tool loop.
    Harness,
    /// No definition; fall back to the custom-registry persona completion.
    RegistryFallback,
}

/// Decides the route for `agent_ref` by consulting the (already-initialised)
/// global `AgentDefinitionRegistry`: a harness definition wins; otherwise the
/// custom-registry fallback. Pure over the global registry so the selection is
/// unit-testable with `init_global_builtins`.
pub(crate) fn route_for_agent_ref(agent_ref: &str) -> AgentRoute {
    if crate::agent::host_agents::resolve(agent_ref).is_some() {
        return AgentRoute::HostAgent;
    }
    let has_definition = crate::agent_registry::host_definition::AgentDefinitionRegistry::current()
        .map(|reg| reg.get(agent_ref).is_some())
        .unwrap_or(false);
    if has_definition {
        AgentRoute::Harness
    } else {
        AgentRoute::RegistryFallback
    }
}

/// Translates a managed tier / `hint:*` / model string into the `default_model`
/// value that routes a freshly-built harness [`Agent`](crate::agent::OpenHumanSessionHost)
/// to the workload serving that tier. The session builder's `provider_role_for`
/// only routes the `hint:<role>` form to a specialised workload, so a bare tier
/// name (`hint:reasoning`) must be normalised to `hint:reasoning` here — otherwise
/// it would silently fall through to the chat workload.
///
/// A **raw/BYOK** model id (e.g. `claude-opus-4`) is instead forwarded verbatim:
/// wrapping it in `hint:chat` would collapse the user's explicit per-node model
/// onto the managed `hint:chat` tier (issue #4598). Left verbatim, it flows through
/// the session builder's generic `chat` role — which inherits
/// `config.default_model` — to `make_openhuman_backend`, which forwards non-tier
/// ids to the backend unchanged. Mirrors the per-node routing
/// [`crate::flows::caps_host::OpenHumanLlm::complete`] applies through the
/// TinyFlows portability contract.
pub(crate) fn harness_model_default_override(node_model: &str) -> String {
    if crate::inference::provider_model::is_raw_model(PROVIDER_OPENHUMAN, node_model) {
        return node_model.to_string();
    }
    format!(
        "hint:{}",
        crate::inference::provider_model::role_for_provider_model(PROVIDER_OPENHUMAN, node_model)
    )
}

/// Decides how to run an `agent_ref` that has no harness definition, given
/// the (already-performed) config-backed custom registry lookup: an
/// [`AgentRoute::Harness`] for a known, *enabled* custom entry — the factory
/// synthesizes a real `AgentDefinition` for it (issue B38/Gap 2), so it can
/// run the full tool loop — and [`AgentRoute::RegistryFallback`] for
/// anything else (no entry at all, or a disabled one, which
/// [`crate::flows::caps_host::agent::OpenHumanAgentRunner::run_via_registry_fallback`] itself rejects with a
/// clear "is disabled" error rather than silently skipping it here). Pure
/// over the lookup result so the decision is unit-testable without a live
/// `Config`/registry.
pub(crate) fn route_custom_entry_lookup(
    entry: Option<&crate::agent_registry::AgentRegistryEntry>,
) -> AgentRoute {
    match entry {
        Some(e) if e.enabled => AgentRoute::Harness,
        _ => AgentRoute::RegistryFallback,
    }
}

#[cfg(test)]
#[path = "agent_route_tests.rs"]
mod tests;
