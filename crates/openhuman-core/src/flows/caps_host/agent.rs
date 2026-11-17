//! The `AgentRunner` capability — running an `agent` node through the harness.
//!
//! Resolves `agent_ref` to a route, decides the model and the run timeout, and
//! builds the prompt the harness receives. The timeout logic is the subtle part:
//! a per-attempt bound has to be scaled against the iteration cap, or a run that
//! is progressing normally gets killed for being long rather than for being
//! stuck.

#![allow(unused_imports)]

use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::config::Config;
use crate::flows::agent_route::{
    harness_model_default_override, route_custom_entry_lookup, route_for_agent_ref, AgentRoute,
};
use crate::modules::tinyflows as module;

#[path = "agent_fallback.rs"]
mod fallback;
#[path = "agent_harness.rs"]
mod harness;

/// [`AgentRunner`] backing an `agent` node's `agent_ref`. It runs the selected
/// agent kind by one of two paths, chosen by [`route_for_agent_ref`]:
///
/// 1. **Full harness turn** (the common case, Phase A). When `agent_ref` names a
///    harness [`AgentDefinition`](crate::agent_registry::host_definition::AgentDefinition),
///    the node builds a real session agent
///    ([`OpenHumanSessionHost::from_config_for_agent`](crate::agent::OpenHumanSessionHost::from_config_for_agent)
///    + `set_agent_definition_name`) and drives one full turn via
///
///    [`OpenHumanSessionHost::run_single`](crate::agent::OpenHumanSessionHost::run_single) — the
///    complete tool loop. The definition's `ToolScope` / `sandbox_mode` /
///    `max_iterations` govern the turn, so an agent node gains its curated
///    toolset with no graph change. This is the same harness pattern
///    `flows_build` / `flows_discover` / cron use, so "every node
///    is a TinyFlows graph" still holds: `run_single` itself routes through the
///    default agent harness, i.e. an agent turn inside the flow graph.
/// 2. **Persona-shaping completion fallback** (no regression for custom agents).
///    When `agent_ref` only resolves to a custom
///    [`AgentRegistryEntry`](crate::agent_registry::AgentRegistryEntry)
///    (no harness definition), the node keeps the original single-completion
///    behavior: the entry's `system_prompt` / `model` are shaped on top of the
///    node request and run through [`OpenHumanLlm::complete`].
///
/// **Security.** No new origin is scoped here: the engine future already runs
/// under the flow's `Workflow` origin (`turn_origin`), so the user's autonomy
/// tier + approval gate apply to the inner turn automatically, and the agent
/// definition's `ToolScope`/sandbox is the inner gate. `agent_ref` is resolved
/// from trusted node config (never model output), so a prompt-injected
/// completion cannot pick an arbitrary agent kind.
///
/// **Per-item cost.** In per-item execution mode the engine calls
/// [`run_agent`](AgentRunner::run_agent) once per input item, so a full harness
/// turn (with memory injection) fans out one `Agent` per item. Since the engine
/// gained bounded per-item concurrency those calls also arrive *simultaneously*,
/// so the host-side guard this doc used to defer is now
/// [`HARNESS_AGENT_SLOTS`] — see it for why the engine's own `concurrency`
/// bound is not enough. Memory injection per node turn is accepted for this
/// first cut (skip-memory is a follow-up).
///
/// **Concurrency safety.** `run_agent` is re-entrant by construction: it builds
/// a fresh [`Agent`](crate::agent::OpenHumanSessionHost) per call and stamps any
/// model override onto a *cloned* `Config`, so concurrent calls never mutate
/// shared state. The origin escalation and approval-run context are task-locals
/// propagated by the engine's `buffer_unordered` (which polls every item on the
/// caller's task), so HITL gating still applies to every fanned-out turn.
pub struct OpenHumanAgentRunner {
    pub config: Arc<Config>,
    /// The context the run acts under, handed over by whoever started it.
    pub ctx: crate::core::runtime::DispatchCtx,
    /// The run's approval context (flow run correlation), handed to the gate
    /// with every acting dispatch.
    pub approval: crate::security::approval::ApprovalTurnContext,
    /// Run-scoped event stream consumed by `CapsHost.AgentEvents`.
    pub progress: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
}

/// Bounded callback state for one in-flight host agent turn.
#[derive(Default)]
pub(crate) struct AgentRunState {
    pub(crate) events: Vec<tinyflows_bus::runs::AgentProgressEvent>,
    pub(crate) outcome: Option<tinyflows_bus::runs::AgentRunOutcome>,
    pub(crate) error: Option<String>,
    pub(crate) turn_stop: Option<tinyagents_harness::turn_stop::TurnStop>,
    pub(crate) done: bool,
}

/// Process-wide ceiling on **simultaneous harness agent turns started by flow
/// nodes**, honoured by [`run_via_harness`](OpenHumanAgentRunner::run_via_harness).
///
/// The engine's per-node `concurrency` bounds one node's fan-out; this bounds
/// the host. Those are different limits and the host one is the load-bearing
/// one: a graph can fan out `concurrency: "all"` over a 200-item array, and
/// several nodes (or several concurrent runs) can be fanning out at once, so
/// without a shared ceiling a single workflow could open hundreds of full agent
/// sessions — each with its own model context and tool loop — and exhaust
/// memory or the inference provider's rate limit.
///
/// Default 8; override with `OPENHUMAN_FLOWS_MAX_PARALLEL_AGENTS`. Waiting on a
/// permit is *not* an error — an over-wide fan-out is throttled to this width
/// rather than rejected, so the workflow still completes, just more slowly.
static HARNESS_AGENT_SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| {
        let permits = max_parallel_harness_agents(
            std::env::var("OPENHUMAN_FLOWS_MAX_PARALLEL_AGENTS")
                .ok()
                .as_deref(),
        );
        tracing::debug!(
            target: "flows",
            permits,
            "[flows] agent_runner: harness concurrency ceiling"
        );
        tokio::sync::Semaphore::new(permits)
    });

/// Default value for [`HARNESS_AGENT_SLOTS`].
const DEFAULT_MAX_PARALLEL_HARNESS_AGENTS: usize = 8;

/// Resolves the harness concurrency ceiling from the raw env-var value.
///
/// A malformed or zero override falls back to the default rather than erroring
/// or, worse, yielding a zero-permit semaphore that would deadlock every flow
/// agent node in the process.
fn max_parallel_harness_agents(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_PARALLEL_HARNESS_AGENTS)
}

impl OpenHumanAgentRunner {
    pub(crate) async fn prepare_agent_run(
        &self,
        request: Value,
        entry_model: Option<&str>,
        system_prompt: Option<&str>,
        effective_iteration_cap: u32,
    ) -> Result<tinyflows_bus::portability::PreparedAgentRun> {
        let proxy = module::portability_proxy(&self.config)
            .await
            .map_err(EngineError::Capability)?;
        proxy
            .call(
                tinyflows_bus::portability::methods::PREPARE_AGENT_RUN,
                &(tinyflows_bus::portability::PrepareAgentRun {
                    request,
                    entry_model: entry_model.map(str::to_owned),
                    system_prompt: system_prompt.map(str::to_owned),
                    effective_iteration_cap,
                },),
            )
            .await
            .map_err(|error| EngineError::Capability(error.to_string()))
    }

    async fn format_agent_result(
        &self,
        agent_ref: &str,
        request: Value,
        final_text: String,
    ) -> Result<Value> {
        let proxy = module::portability_proxy(&self.config)
            .await
            .map_err(EngineError::Capability)?;
        proxy
            .call(
                tinyflows_bus::portability::methods::FORMAT_AGENT_RUN,
                &(tinyflows_bus::portability::FormatAgentRun {
                    agent_ref: agent_ref.to_owned(),
                    request,
                    final_text,
                },),
            )
            .await
            .map_err(|error| EngineError::Capability(error.to_string()))
    }

    pub(crate) async fn run_agent(
        &self,
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        // Entry point of a flow agent node: the run's own context.
        let dispatch = self.ctx.clone();
        // The harness definition registry must be initialised before we can
        // build a named agent. Idempotent: a booted core already did this at
        // startup; a bare flow run (tests, standalone) has not. A failure here
        // is non-fatal — we log and fall through to the registry-entry route.
        if let Err(e) = crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global(
            &self.config.workspace_dir,
        ) {
            tracing::warn!(
                target: "flows",
                agent_ref,
                error = %e,
                "[flows] agent_runner: agent definition registry init failed — will attempt the \
                 custom registry-entry fallback"
            );
        }

        match route_for_agent_ref(agent_ref) {
            AgentRoute::HostAgent => {
                // Resolved again rather than carried by the route so the route
                // stays a plain value; a host that dropped the agent in between
                // degrades to the registry build below.
                let host = crate::agent::host_agents::resolve(agent_ref);
                tracing::info!(
                    target: "flows",
                    agent_ref,
                    "[flows] agent_runner: HOST AGENT path — running the host-registered agent \
                     with its own tools in its own context"
                );
                self.run_via_harness(&dispatch, agent_ref, request, conn, None, host)
                    .await
            }
            AgentRoute::Harness => {
                tracing::info!(
                    target: "flows",
                    agent_ref,
                    "[flows] agent_runner: HARNESS path — running the full agent tool loop"
                );
                // A shipped/TOML harness definition has no `entry.model` — the
                // definition's own `ModelSpec` (already applied by the session
                // builder) is the only model pin in play here.
                self.run_via_harness(&dispatch, agent_ref, request, conn, None, None)
                    .await
            }
            AgentRoute::RegistryFallback => {
                // `route_for_agent_ref` only consults the harness
                // `AgentDefinitionRegistry`, so a miss there used to mean
                // "run the persona-only completion fallback" unconditionally
                // — even for a user-created custom agent, which has real
                // `tool_allowlist`/`model` settings that fallback ignores.
                //
                // The agent factory (`OpenHumanSessionHost::from_config_for_agent`) now
                // also consults `config.agent_registry.entries` on a
                // harness-registry miss and synthesizes a real
                // `AgentDefinition` for any `AgentRegistrySource::Custom`
                // entry it finds (issue B38/Gap 2). So: route a *known,
                // enabled* custom entry through the harness turn — it gets
                // its real tool belt — and reserve the persona-only
                // completion for `agent_ref`s that are unknown to both the
                // harness registry AND the custom config registry (or that
                // are disabled, which `run_via_registry_fallback` already
                // rejects with a clear error).
                let custom_entry =
                    crate::agent_registry::find_custom_in_config(&self.config, agent_ref);
                let entry_model = custom_entry.as_ref().and_then(|e| e.model.clone());
                match route_custom_entry_lookup(custom_entry.as_ref()) {
                    AgentRoute::Harness | AgentRoute::HostAgent => {
                        tracing::info!(
                            target: "flows",
                            agent_ref,
                            "[flows] agent_runner: CUSTOM-REGISTRY path — routing through the \
                             harness so the custom agent runs with its real tool belt instead of \
                             the persona-only completion fallback"
                        );
                        // Preserve the custom entry's own `model` pin (e.g.
                        // `hint:reasoning` or a raw BYOK model id) as the
                        // fallback below the node's own override — same
                        // precedence `run_via_registry_fallback` already gave
                        // it, now honored on the harness path too (P2 review
                        // comment on this PR: this previously regressed to the
                        // default chat model for a custom flow agent with no
                        // per-node override).
                        self.run_via_harness(
                            &dispatch,
                            agent_ref,
                            request,
                            conn,
                            entry_model.as_deref(),
                            None,
                        )
                        .await
                    }
                    AgentRoute::RegistryFallback => {
                        tracing::info!(
                            target: "flows",
                            agent_ref,
                            "[flows] agent_runner: FALLBACK path — persona-shaping single \
                             completion for a custom registry entry"
                        );
                        self.run_via_registry_fallback(&dispatch, agent_ref, request, conn)
                            .await
                    }
                }
            }
        }
    }
}

impl OpenHumanAgentRunner {}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
