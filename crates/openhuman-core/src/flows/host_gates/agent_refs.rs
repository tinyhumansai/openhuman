use super::graph_json::nodes;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use serde_json::Value;

// Agent-ref resolvability gate: an `agent` node's `agent_ref` must name a
// real agent, not the runtime's `RegistryFallback` "unknown agent_ref" case
// ─────────────────────────────────────────────────────────────────────────────
//
// `run_via_registry_fallback` (`flows/caps_host/agent.rs`) hard-errors mid-run with
// "unknown agent_ref '…'" the moment an `agent` node's `config.agent_ref`
// doesn't resolve to either a harness `AgentDefinition` or a custom agent
// registry entry. Today that is the FIRST time an author finds out — the
// graph proposes, saves, and even passes every other builder gate, then
// fails on the very node whose whole job was to run. This gate moves that
// same check to propose/edit/save time so a broken `agent_ref` is rejected
// before it's ever persisted, using the exact resolution the runtime uses
// (`route_for_agent_ref` + `agent_registry::get_agent`) rather than
// re-implementing it.
//
// A plain `agent` node with NO `agent_ref` is unaffected (and must stay
// that way) — it runs on the default LLM completion (`caps.llm`), never
// touches `OpenHumanAgentRunner`'s routing at all, so there is nothing to
// resolve.

/// Rejects an `agent` node whose `config.agent_ref` would hit the runtime's
/// `RegistryFallback` "unknown agent_ref" hard error mid-run
/// (`run_via_registry_fallback` in `flows/caps_host/agent.rs`) — a real ref is one
/// that resolves via [`crate::flows::agent_route::route_for_agent_ref`]
/// to a harness [`AgentDefinition`](crate::agent_registry::host_definition::AgentDefinition)
/// (`AgentRoute::Harness`), OR — when it routes to `AgentRoute::RegistryFallback`
/// — resolves to an *enabled*
/// [`AgentRegistryEntry`](crate::agent_registry::AgentRegistryEntry)
/// via [`crate::agent_registry::get_agent`]. Both are exactly the
/// checks `OpenHumanAgentRunner::run_agent` performs at run time, reused here
/// rather than duplicated so the two planes cannot drift.
///
/// A node with no `agent_ref` (or a blank one) is a plain agent node — it
/// runs on the default LLM completion, never reaches this routing at all —
/// and is skipped, not rejected. A registry lookup failure (e.g. config
/// unavailable) fails OPEN (skipped, logged) like the sibling
/// `validate_connection_refs` gate: this gate must never false-reject a
/// graph because of a transient local read.
///
/// Takes `config` for two reasons. First (CodeRabbit/Codex review on #5114):
/// one-shot contexts — the generic `openhuman <namespace> <function>` CLI
/// dispatcher (`default_state()`, no bootstrap), cron, tests — may reach this
/// gate before the full server bootstrap has called
/// [`AgentDefinitionRegistry::init_global`]. Without it, `route_for_agent_ref`
/// sees an empty global registry and routes EVERY ref — including a real
/// workspace-TOML harness definition — to `RegistryFallback`, which then only
/// checks the custom agent registry and would reject a valid harness agent
/// as unknown. So this gate defensively (re-)initialises the harness registry
/// itself, same idempotent (`OnceLock`) idiom as
/// other lazy registry users, before resolving any ref — the two planes
/// (author-time gate and `OpenHumanAgentRunner::run_agent` at actual run
/// time) then always see the same registry state. Second, it threads through
/// to `agent_registry::get_agent`'s underlying config load.
///
/// Also lazily caches the custom agent registry snapshot on the first
/// `RegistryFallback` node (CodeRabbit nitpick): a graph with several
/// non-harness `agent_ref`s previously triggered one `config_rpc::
/// load_config_with_timeout` per node; an all-`Harness`/no-custom-ref graph
/// still never reads it at all.
pub(crate) async fn validate_agent_refs(
    ctx: &DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Vec<String> {
    use crate::agent_registry::host_definition::AgentDefinitionRegistry;
    use crate::agent_registry::AgentRegistryEntry;
    use crate::flows::agent_route::{route_for_agent_ref, AgentRoute};

    let mut errors = Vec::new();
    let mut harness_registry_init_attempted = false;
    let mut custom_registry: Option<Result<Vec<AgentRegistryEntry>, String>> = None;

    for node in nodes(graph) {
        if node.kind != "agent" {
            continue;
        }
        let Some(agent_ref) = node.config.get("agent_ref").and_then(Value::as_str) else {
            continue;
        };
        let agent_ref = agent_ref.trim();
        if agent_ref.is_empty() {
            continue;
        }

        if !harness_registry_init_attempted && AgentDefinitionRegistry::global().is_none() {
            harness_registry_init_attempted = true;
            if let Err(e) = AgentDefinitionRegistry::init_global(&config.workspace_dir) {
                tracing::debug!(
                    target: "flows",
                    error = %e,
                    "[flows] agent-ref check: harness registry init failed — falling through \
                     to route resolution with whatever state is available"
                );
            }
        }

        match route_for_agent_ref(agent_ref) {
            AgentRoute::Harness | AgentRoute::HostAgent => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %agent_ref,
                    "[flows] agent-ref check: resolves to a harness or host agent definition"
                );
            }
            AgentRoute::RegistryFallback => {
                if custom_registry.is_none() {
                    custom_registry = Some(crate::agent_registry::list_agents(ctx, true).await);
                }
                match custom_registry.as_ref().expect("just populated") {
                    Ok(entries) => match entries.iter().find(|entry| entry.id == agent_ref) {
                        Some(entry) if entry.enabled => {
                            tracing::debug!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: resolves to an enabled custom agent \
                                 registry entry"
                            );
                        }
                        Some(_disabled) => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: agent_ref is registered but disabled — \
                                 rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is registered but currently \
                                 disabled — enable it (or pick another agent_ref via \
                                 list_agent_definitions) before this node can run.",
                                node.id
                            ));
                        }
                        None => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: unknown agent_ref — neither a harness \
                                 definition nor a custom agent registry entry — rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is not a real agent — it \
                                 names neither a built-in agent definition nor a custom agent \
                                 registry entry, and would fail at run time with an \"unknown \
                                 agent_ref\" error. Call list_agent_definitions to see the real, \
                                 selectable agent_ref values.",
                                node.id
                            ));
                        }
                    },
                    Err(e) => {
                        tracing::debug!(
                            target: "flows",
                            node = %node.id,
                            %agent_ref,
                            error = %e,
                            "[flows] agent-ref check: custom agent registry lookup unavailable — \
                             skipping (fail-open)"
                        );
                    }
                }
            }
        }
    }
    errors
}
