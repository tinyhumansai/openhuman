//! The `AgentRunner` capability — running an `agent` node through the harness.
//!
//! Resolves `agent_ref` to a route, decides the model and the run timeout, and
//! builds the prompt the harness receives. The timeout logic is the subtle part:
//! a per-attempt bound has to be scaled against the iteration cap, or a run that
//! is progressing normally gets killed for being long rather than for being
//! stuck.

#![allow(unused_imports)]

use std::borrow::Cow;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::config::Config;
use crate::inference::provider::{is_raw_passthrough_model, role_for_model_tier};

/// [`AgentRunner`] backing an `agent` node's `agent_ref`. It runs the selected
/// agent kind by one of two paths, chosen by [`route_for_agent_ref`]:
///
/// 1. **Full harness turn** (the common case, Phase A). When `agent_ref` names a
///    harness [`AgentDefinition`](crate::agent::harness::host_definition::AgentDefinition),
///    the node builds a real session agent
///    ([`OpenHumanSessionHost::from_config_for_agent`](crate::agent::OpenHumanSessionHost::from_config_for_agent)
///    + `set_agent_definition_name`) and drives one full turn via
///
///    [`OpenHumanSessionHost::run_single`](crate::agent::OpenHumanSessionHost::run_single) — the
///    complete tool loop. The definition's `ToolScope` / `sandbox_mode` /
///    `max_iterations` govern the turn, so an agent node gains its curated
///    toolset with no graph change. This is the same harness pattern
///    `flows_build` / `flows_discover` / cron use, so "every node
///    is a tinyagents graph" still holds: `run_single` itself routes through the
///    default agent graph, i.e. a nested tinyagents graph (the agent turn) inside
///    the flow's tinyagents graph.
/// 2. **Persona-shaping completion fallback** (no regression for custom agents).
///    When `agent_ref` only resolves to a custom
///    [`AgentRegistryEntry`](crate::agent::registry::AgentRegistryEntry)
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

/// Which execution path an `agent_ref` routes to (see [`OpenHumanAgentRunner`]).
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
    let has_definition = crate::agent::harness::host_definition::AgentDefinitionRegistry::current()
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
/// [`OpenHumanLlm::complete`] applies via `agent_prompt::resolve_completion_model`.
pub(crate) fn harness_model_default_override(node_model: &str) -> String {
    if is_raw_passthrough_model(node_model) {
        return node_model.to_string();
    }
    format!("hint:{}", role_for_model_tier(node_model))
}

#[async_trait]
impl AgentRunner for OpenHumanAgentRunner {
    async fn run_agent(
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
        if let Err(e) = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global(
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
                    crate::agent::registry::find_custom_in_config(&self.config, agent_ref);
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

/// Decides how to run an `agent_ref` that has no harness definition, given
/// the (already-performed) config-backed custom registry lookup: an
/// [`AgentRoute::Harness`] for a known, *enabled* custom entry — the factory
/// synthesizes a real `AgentDefinition` for it (issue B38/Gap 2), so it can
/// run the full tool loop — and [`AgentRoute::RegistryFallback`] for
/// anything else (no entry at all, or a disabled one, which
/// [`OpenHumanAgentRunner::run_via_registry_fallback`] itself rejects with a
/// clear "is disabled" error rather than silently skipping it here). Pure
/// over the lookup result so the decision is unit-testable without a live
/// `Config`/registry.
pub(crate) fn route_custom_entry_lookup(
    entry: Option<&crate::agent::registry::AgentRegistryEntry>,
) -> AgentRoute {
    match entry {
        Some(e) if e.enabled => AgentRoute::Harness,
        _ => AgentRoute::RegistryFallback,
    }
}

impl OpenHumanAgentRunner {
    /// Full harness turn: build a real session agent for `agent_ref` and drive
    /// one `run_single` under the node's model override + timeout. See
    /// [`OpenHumanAgentRunner`] for the security/origin contract.
    ///
    /// `entry_model` is the custom `AgentRegistryEntry`'s own `model` pin (a
    /// `hint:<role>` or raw BYOK model id), when `agent_ref` resolved to one —
    /// `None` for a shipped/TOML harness definition, which has no such entry.
    /// It is the fallback below the node's own `config.model` override (see
    /// `resolve_node_model`), matching the precedence
    /// `run_via_registry_fallback` already gave `entry.model` — without this,
    /// a custom flow agent's model pin (e.g. `hint:reasoning`) silently
    /// dropped to the default chat model once routed through the harness.
    ///
    /// **Synchronous only (B40 / Gap 4).** A flow `agent` node runs here with
    /// no explicit chat thread on its run context. If the
    /// agent it runs is a delegating agent (e.g. the orchestrator) and
    /// calls `spawn_async_subagent` directly, the tool now refuses (see the
    /// `parent_thread_id.is_none()` guard in
    /// `agent_orchestration::tools::spawn_async_subagent`) rather than
    /// silently discarding the background result once it finishes —
    /// `background_delivery` has nowhere to deliver it without a thread id.
    /// This module does not bridge async subagent delivery into flow runs.
    /// For work that needs to happen in parallel, model it as parallel flow
    /// nodes (the tinyflows engine already fans those out) instead of
    /// reaching for a background sub-agent from inside a single node.
    async fn run_via_harness(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
        entry_model: Option<&str>,
        host: Option<crate::agent::host_agents::HostAgent>,
    ) -> Result<Value> {
        use crate::agent::OpenHumanSessionHost;

        // Hold a slot for the whole turn: a fanned-out node can call this
        // hundreds of times at once, and each call below builds a full agent
        // session. Acquired before any work so waiters queue rather than pile
        // up half-built agents. `_permit` is released on drop at end of scope,
        // including on every early return below.
        let _permit = HARNESS_AGENT_SLOTS.acquire().await.map_err(|_| {
            EngineError::Capability(
                "agent node: harness concurrency limiter closed unexpectedly".to_string(),
            )
        })?;

        if let Some(c) = conn {
            tracing::debug!(
                target: "flows",
                conn = %c,
                "[flows] agent_runner: connection_ref present but not resolved to a BYOK account \
                 for the harness turn (matches OpenHumanLlm)"
            );
        }

        // Model precedence for a harness node: node `config.model` > the
        // custom registry entry's own `model` pin (if any) > the definition's
        // own default.
        let node_model = resolve_node_model(&request, entry_model);

        // Apply the override the cron way (`run_agent_job`): a cloned `Config`
        // with a new `default_model`, so we never mutate the shared config or
        // invent a new Agent setter API. The tier is normalised to the
        // `hint:<role>` form the session builder routes on.
        // A host-registered agent starts from its own config (its provider
        // model and route), not the flow runner's.
        let base: &Config = host
            .as_ref()
            .map_or(self.config.as_ref(), |host| &host.config);
        let effective: Cow<'_, Config> = match node_model.as_deref() {
            Some(model) => {
                let mut config = base.clone();
                config.default_model = Some(harness_model_default_override(model));
                Cow::Owned(config)
            }
            None => Cow::Borrowed(base),
        };

        let built = match &host {
            Some(host) => host.session_host(effective.as_ref(), None),
            None => {
                OpenHumanSessionHost::from_config_for_agent(dispatch, effective.as_ref(), agent_ref)
            }
        };
        let mut agent = built.map_err(|e| {
            EngineError::Capability(format!(
                "agent node: failed to build harness agent '{agent_ref}': {e:#}"
            ))
        })?;
        agent.set_agent_definition_name(agent_ref.to_string());

        let prompt = build_harness_run_prompt(&request);

        let requested_timeout_secs = request.get("timeout_secs").and_then(Value::as_u64);
        let base_timeout_secs = clamp_run_timeout_secs(requested_timeout_secs);

        // Issue #4868 — the session builder now stamps `agent_ref`'s own
        // `effective_max_iterations()` onto the agent (instead of the global
        // default of 10), so `code_executor`/`tools_agent`/etc. can run up to
        // 50 iterations here. Read the cap actually applied to `agent`
        // (reflects the definition cap or the global fallback, whichever the
        // builder resolved) and scale the DEFAULT timeout accordingly — see
        // `scale_timeout_for_iteration_cap`.
        //
        // Post-merge Codex P2 finding: an EXPLICIT `timeout_secs` the node
        // config supplied is a caller-chosen bound (e.g. a fast-fail/SLA of
        // 120s) and must be honored as-is, never scaled up just because the
        // agent's iteration cap is high — see `resolve_run_timeout_secs`.
        let effective_iteration_cap = agent.agent_config().max_tool_iterations;
        let timeout_secs =
            resolve_run_timeout_secs(requested_timeout_secs, effective_iteration_cap);

        tracing::debug!(
            target: "flows",
            agent_ref,
            node_model = node_model.as_deref().unwrap_or("<definition-default>"),
            default_model = effective.default_model.as_deref().unwrap_or("<config-default>"),
            effective_iteration_cap,
            explicit_timeout_secs = requested_timeout_secs.is_some(),
            base_timeout_secs,
            timeout_secs,
            prompt_len = prompt.len(),
            "[flows] agent_runner: dispatching full harness turn"
        );

        // Nested-harness HITL escalation (issue #4595): the engine future runs
        // under the flow's Workflow origin, but the flow author only pre-
        // declared `agent_ref` — not the concrete tools the harness LLM will
        // pick from the definition's `ToolScope`. If we let the inner turn
        // inherit a `Workflow { require_approval: false }` origin,
        // `ApprovalGate::intercept_audited` treats it as a trust root and
        // auto-`Allow`s external_effect tools (see
        // `crates/openhuman-core/src/security/approval/gate.rs` `Workflow { require_approval: false }`
        // branch), which would let a scheduled / app-event flow reach out to
        // Slack / email / desktop control with no HITL. We force
        // `require_approval: true` around `run_single` so external_effect tools
        // park for a real decision the same way flow acting nodes escalated by
        // [`gate_call_for_tier`] do. Read-only tools (no `external_effect`)
        // aren't gated by `intercept_audited` at all, so this doesn't add noise
        // for pure-read nested agents.
        //
        // Cancellation: the run_registry token aborts the engine future, and the
        // inner turn drops with it (task-local scope unwinds cleanly).
        use crate::agent::turn_origin;
        let escalated_origin = escalated_origin_for_nested_harness(self.ctx.turn_origin());
        if let Some(ref escalated) = escalated_origin {
            tracing::debug!(
                target: "flows",
                agent_ref,
                origin = ?escalated,
                "[flows] agent_runner: escalating nested harness turn to Workflow{{require_approval:true}} \
                 so external_effect tools park for HITL (issue #4595)"
            );
        }
        let run: std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
        > = if let Some(escalated) = escalated_origin {
            Box::pin(turn_origin::with_origin(
                escalated,
                agent.run_single_with_approval(&prompt, None, self.approval.clone()),
            ))
        } else {
            Box::pin(agent.run_single_with_approval(&prompt, None, self.approval.clone()))
        };
        // A host agent's turn reads its own context (config, domains, tool
        // groups, session store); the origin task-local is untouched.
        let run: std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
        > = match host
            .as_ref()
            .map(|host| std::sync::Arc::clone(&host.context))
        {
            Some(context) => Box::pin(crate::core::runtime::CoreContext::scope(context, run)),
            None => run,
        };
        let final_text =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), run).await {
                Ok(Ok(text)) => text,
                Ok(Err(e)) => {
                    tracing::warn!(
                        target: "flows",
                        agent_ref,
                        error = %e,
                        "[flows] agent_runner: harness turn failed"
                    );
                    return Err(EngineError::Capability(format!(
                        "agent node '{agent_ref}' turn failed: {e:#}"
                    )));
                }
                Err(_) => {
                    tracing::warn!(
                        target: "flows",
                        agent_ref,
                        timeout_secs,
                        "[flows] agent_runner: harness turn timed out"
                    );
                    return Err(EngineError::Capability(format!(
                        "agent node '{agent_ref}' timed out after {timeout_secs}s"
                    )));
                }
            };

        Ok(build_agent_result(agent_ref, &final_text, &request))
    }

    /// Persona-shaping single-completion fallback for a custom
    /// [`AgentRegistryEntry`](crate::agent::registry::AgentRegistryEntry)
    /// with no harness definition — the pre-Phase-A behavior, kept so custom
    /// agents don't regress.
    async fn run_via_registry_fallback(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        // Resolve + validate the requested agent kind against the registry.
        let entry = crate::agent::registry::get_agent(dispatch, agent_ref)
            .await
            .map_err(EngineError::Capability)?
            .ok_or_else(|| {
                EngineError::Capability(format!(
                "agent node: unknown agent_ref '{agent_ref}' (neither a harness definition nor \
                     a custom agent registry entry)"
            ))
            })?;
        if !entry.enabled {
            return Err(EngineError::Capability(format!(
                "agent node: agent_ref '{agent_ref}' is disabled"
            )));
        }

        tracing::debug!(
            target: "flows",
            agent_ref,
            has_system_prompt = entry.system_prompt.is_some(),
            model = entry.model.as_deref().unwrap_or("<role-default>"),
            "[flows] agent_runner: applying custom registered agent-kind persona to the completion"
        );

        // Shape the completion by the agent kind: prepend the agent's system
        // prompt (its persona) ahead of the node's messages, and adopt its model
        // when the node didn't pin one. The completion itself runs through the
        // same provider path as a plain agent turn (OpenHumanLlm::complete), so
        // structured-output / envelope behavior is identical.
        let mut request = request;
        if let Some(system_prompt) = entry.system_prompt.as_deref().filter(|s| !s.is_empty()) {
            prepend_system_message(&mut request, system_prompt);
        }
        if let Some(model) = entry.model.as_deref().filter(|s| !s.is_empty()) {
            if request.get("model").and_then(Value::as_str).is_none() {
                if let Value::Object(map) = &mut request {
                    map.insert("model".to_string(), Value::String(model.to_string()));
                }
            }
        }

        OpenHumanLlm {
            ctx: self.ctx.clone(),
            config: self.config.clone(),
        }
        .complete(request, conn)
        .await
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
