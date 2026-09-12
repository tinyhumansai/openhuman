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
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::{is_raw_passthrough_model, role_for_model_tier};

/// [`AgentRunner`] backing an `agent` node's `agent_ref`. It runs the selected
/// agent kind by one of two paths, chosen by [`route_for_agent_ref`]:
///
/// 1. **Full harness turn** (the common case, Phase A). When `agent_ref` names a
///    harness [`AgentDefinition`](crate::openhuman::agent::harness::definition::AgentDefinition),
///    the node builds a real session agent
///    ([`Agent::from_config_for_agent`](crate::openhuman::agent::Agent::from_config_for_agent)
///    + `set_agent_definition_name`) and drives one full turn via
///
///    [`Agent::run_single`](crate::openhuman::agent::Agent::run_single) — the
///    complete tool loop. The definition's `ToolScope` / `sandbox_mode` /
///    `max_iterations` govern the turn, so an agent node gains its curated
///    toolset with no graph change. This is the same harness pattern
///    `flows_build` / `flows_discover` / cron / subconscious use, so "every node
///    is a tinyagents graph" still holds: `run_single` itself routes through the
///    default agent graph, i.e. a nested tinyagents graph (the agent turn) inside
///    the flow's tinyagents graph.
/// 2. **Persona-shaping completion fallback** (no regression for custom agents).
///    When `agent_ref` only resolves to a custom
///    [`AgentRegistryEntry`](crate::openhuman::agent::registry::AgentRegistryEntry)
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
/// a fresh [`Agent`](crate::openhuman::agent::Agent) per call and stamps any
/// model override onto a *cloned* `Config`, so concurrent calls never mutate
/// shared state. The origin escalation and approval-run context are task-locals
/// propagated by the engine's `buffer_unordered` (which polls every item on the
/// caller's task), so HITL gating still applies to every fanned-out turn.
pub struct OpenHumanAgentRunner {
    pub config: Arc<Config>,
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
    let has_definition =
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
            .map(|reg| reg.get(agent_ref).is_some())
            .unwrap_or(false);
    if has_definition {
        AgentRoute::Harness
    } else {
        AgentRoute::RegistryFallback
    }
}

/// The wall-clock timeout for one agent-node harness turn: the node's requested
/// `timeout_secs` clamped to `10..=600`, defaulting to `240` when unset. A hung
/// provider/tool call must never wedge the flow run.
pub(crate) fn clamp_run_timeout_secs(requested: Option<u64>) -> u64 {
    requested.map(|s| s.clamp(10, 600)).unwrap_or(240)
}

/// Issue #4868 — scale `base_timeout_secs` up for agents whose effective
/// iteration cap exceeds the (until now, universal) global default of 10.
///
/// A `tools_agent`/`code_executor`/etc. node now legitimately runs up to 50
/// iterations (`iteration_policy = "extended"`). At a worst case of
/// ~10s/iteration that's ~500s, comfortably exceeding the 240s
/// `clamp_run_timeout_secs` default — the node would be killed by timeout
/// before it could use its own declared budget. Agents whose effective cap is
/// still at or below the old global default (10) are unaffected and keep the
/// unscaled `base_timeout_secs`. The scaled floor is capped at the existing
/// 600s maximum `clamp_run_timeout_secs` already enforces, so this can only
/// ever raise the effective timeout up to that ceiling, never past it.
pub(crate) fn scale_timeout_for_iteration_cap(
    base_timeout_secs: u64,
    effective_iteration_cap: usize,
) -> u64 {
    if effective_iteration_cap > 10 {
        let scaled = (effective_iteration_cap as u64).saturating_mul(12).min(600);
        base_timeout_secs.max(scaled)
    } else {
        base_timeout_secs
    }
}

/// Resolves the actual wall-clock timeout for one agent-node harness turn,
/// combining [`clamp_run_timeout_secs`] and [`scale_timeout_for_iteration_cap`]
/// per the post-merge Codex P2 finding on issue #4868's iteration-cap timeout
/// scaling: **an explicit `timeout_secs` the flow author set on the node must
/// never be scaled up.**
///
/// A node's `timeout_secs` can be an intentional fast-fail/SLA bound (e.g.
/// `timeout_secs: 120` to bound a health-check-style agent call) — scaling
/// that up to match a 50-iteration-cap agent would silently defeat the
/// author's explicit choice. So the iteration-cap scaling only ever widens
/// the *default* (no `timeout_secs` supplied) 240s bound; an explicit value is
/// clamped to `10..=600` (as it always was) and returned as-is.
///
/// `requested_timeout_secs` is the raw `request["timeout_secs"]` (before
/// clamping) so this function can distinguish "caller supplied a value" from
/// "caller supplied nothing" — [`clamp_run_timeout_secs`] alone collapses that
/// distinction into a plain `u64`.
pub(crate) fn resolve_run_timeout_secs(
    requested_timeout_secs: Option<u64>,
    effective_iteration_cap: usize,
) -> u64 {
    let base_timeout_secs = clamp_run_timeout_secs(requested_timeout_secs);
    if requested_timeout_secs.is_some() {
        base_timeout_secs
    } else {
        scale_timeout_for_iteration_cap(base_timeout_secs, effective_iteration_cap)
    }
}

/// Renders an agent-node completion `request` into the single user message
/// [`Agent::run_single`](crate::openhuman::agent::Agent::run_single) takes: the
/// `prompt` string when present and non-empty, else the `messages` array
/// flattened to `"<role>: <content>"` lines (blank entries skipped). Empty
/// string when neither yields content. Mirrors how [`OpenHumanLlm::complete`]
/// reads `prompt`/`messages`, collapsed to one string because the harness turn
/// entry point is single-message.
pub(crate) fn node_request_to_prompt(request: &Value) -> String {
    if let Some(prompt) = request.get("prompt").and_then(Value::as_str) {
        let prompt = prompt.trim();
        if !prompt.is_empty() {
            return prompt.to_string();
        }
    }
    if let Some(entries) = request.get("messages").and_then(Value::as_array) {
        let parts: Vec<String> = entries
            .iter()
            .filter_map(|entry| {
                let content = entry.get("content").and_then(Value::as_str)?.trim();
                if content.is_empty() {
                    return None;
                }
                let role = entry.get("role").and_then(Value::as_str).unwrap_or("user");
                Some(format!("{role}: {content}"))
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("\n\n");
        }
    }
    String::new()
}

/// Model precedence for an agent node, returning the raw model string as
/// written:
/// 1. node `config.model` — a managed tier (`reasoning-v1`, `chat-v1`, …) or a
///    `hint:*` alias;
/// 2. the registry `entry_model` (custom agents);
/// 3. `None` — no override, so the harness definition's / role default stands.
///
/// Routing translation (tier → workload) happens at application time via
/// [`harness_model_default_override`]; this function is only the precedence pick,
/// so it stays config-free and trivially testable.
pub(crate) fn resolve_node_model(request: &Value, entry_model: Option<&str>) -> Option<String> {
    if let Some(node_model) = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        return Some(node_model.to_string());
    }
    entry_model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
}

/// Translates a managed tier / `hint:*` / model string into the `default_model`
/// value that routes a freshly-built harness [`Agent`](crate::openhuman::agent::Agent)
/// to the workload serving that tier. The session builder's `provider_role_for`
/// only routes the `hint:<role>` form to a specialised workload, so a bare tier
/// name (`reasoning-v1`) must be normalised to `hint:reasoning` here — otherwise
/// it would silently fall through to the chat workload.
///
/// A **raw/BYOK** model id (e.g. `claude-opus-4`) is instead forwarded verbatim:
/// wrapping it in `hint:chat` would collapse the user's explicit per-node model
/// onto the managed `chat-v1` tier (issue #4598). Left verbatim, it flows through
/// the session builder's generic `chat` role — which inherits
/// `config.default_model` — to `make_openhuman_backend`, which forwards non-tier
/// ids to the backend unchanged. Mirrors the per-node routing
/// [`OpenHumanLlm::complete`] applies via [`resolve_completion_model`].
pub(crate) fn harness_model_default_override(node_model: &str) -> String {
    if is_raw_passthrough_model(node_model) {
        return node_model.to_string();
    }
    format!("hint:{}", role_for_model_tier(node_model))
}

/// Builds the JSON-steering instruction that a structured-output node needs (an
/// `output_parser.schema` or `response_format: "json"`), or `None` when the node
/// didn't request structured output. Shared shape with
/// [`OpenHumanLlm::complete`]'s inline steering; the harness path appends it to
/// the run prompt (rather than inserting a system message) because `run_single`
/// takes a single user message.
pub(crate) fn structured_output_instruction(request: &Value) -> Option<String> {
    if !structured_output_requested(request) {
        return None;
    }
    let mut instruction = "Respond with a single JSON object only — no prose, no \
                           markdown code fences."
        .to_string();
    if let Some(schema) = request
        .get("output_parser")
        .and_then(|p| p.get("schema"))
        .filter(|s| !s.is_null())
    {
        instruction.push_str(&format!(
            " The object must match this JSON Schema:\n{schema}"
        ));
    }
    Some(instruction)
}

/// Builds [`OpenHumanAgentRunner::run_via_harness`]'s single run message: the
/// node's `input_context` (when present — see [`input_context_block`]'s doc),
/// then the JSON-steering instruction (when the node requested structured
/// output), then the node's own prompt (or flattened messages, via
/// [`node_request_to_prompt`]). Each present part is separated by a blank
/// line; an absent part contributes nothing (no stray blank lines). Pulled
/// out as its own pure function — rather than inlined in `run_via_harness` —
/// so the prepend order is unit-testable without building a real harness
/// [`Agent`](crate::openhuman::agent::Agent).
pub(crate) fn build_harness_run_prompt(request: &Value) -> String {
    let parts = [
        input_context_block(request),
        structured_output_instruction(request),
        Some(node_request_to_prompt(request)).filter(|p| !p.is_empty()),
    ];
    parts.into_iter().flatten().collect::<Vec<_>>().join("\n\n")
}

/// Shapes an agent-node harness turn's final text into the node's output value,
/// mirroring [`OpenHumanLlm::complete`]: when the node requested structured
/// output and the text parses as JSON, the parsed object/array is returned so
/// downstream `=item.<field>` / `=nodes.<id>.item.<field>` bindings work;
/// otherwise `{ text, agent_ref }`. The vendor `agent` node then folds this into
/// the stable `{ json, text, raw }` envelope, and the `output_parser` sub-port
/// still applies.
pub(crate) fn build_agent_result(agent_ref: &str, final_text: &str, request: &Value) -> Value {
    if structured_output_requested(request) {
        if let Some(parsed) = extract_structured_json(final_text) {
            tracing::debug!(
                target: "flows",
                agent_ref,
                "[flows] agent_runner: structured output extracted from harness turn"
            );
            return parsed;
        }
        tracing::warn!(
            target: "flows",
            agent_ref,
            "[flows] agent_runner: structured output requested but none of the extraction strategies \
             produced valid JSON — falling back to the {{text}} shape (the output_parser sub-port may \
             still coerce it)"
        );
    }
    json!({ "text": final_text, "agent_ref": agent_ref })
}

#[async_trait]
impl AgentRunner for OpenHumanAgentRunner {
    async fn run_agent(
        &self,
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        // The harness definition registry must be initialised before we can
        // build a named agent. Idempotent: a booted core already did this at
        // startup; a bare flow run (tests, standalone) has not. A failure here
        // is non-fatal — we log and fall through to the registry-entry route.
        if let Err(e) =
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global(
                &self.config.workspace_dir,
            )
        {
            tracing::warn!(
                target: "flows",
                agent_ref,
                error = %e,
                "[flows] agent_runner: agent definition registry init failed — will attempt the \
                 custom registry-entry fallback"
            );
        }

        match route_for_agent_ref(agent_ref) {
            AgentRoute::Harness => {
                tracing::info!(
                    target: "flows",
                    agent_ref,
                    "[flows] agent_runner: HARNESS path — running the full agent tool loop"
                );
                // A shipped/TOML harness definition has no `entry.model` — the
                // definition's own `ModelSpec` (already applied by the session
                // builder) is the only model pin in play here.
                self.run_via_harness(agent_ref, request, conn, None).await
            }
            AgentRoute::RegistryFallback => {
                // `route_for_agent_ref` only consults the harness
                // `AgentDefinitionRegistry`, so a miss there used to mean
                // "run the persona-only completion fallback" unconditionally
                // — even for a user-created custom agent, which has real
                // `tool_allowlist`/`model` settings that fallback ignores.
                //
                // The agent factory (`Agent::from_config_for_agent`) now
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
                let custom_entry = crate::openhuman::agent::registry::find_custom_in_config(
                    &self.config,
                    agent_ref,
                );
                let entry_model = custom_entry.as_ref().and_then(|e| e.model.clone());
                match route_custom_entry_lookup(custom_entry.as_ref()) {
                    AgentRoute::Harness => {
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
                        self.run_via_harness(agent_ref, request, conn, entry_model.as_deref())
                            .await
                    }
                    AgentRoute::RegistryFallback => {
                        tracing::info!(
                            target: "flows",
                            agent_ref,
                            "[flows] agent_runner: FALLBACK path — persona-shaping single \
                             completion for a custom registry entry"
                        );
                        self.run_via_registry_fallback(agent_ref, request, conn)
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
    entry: Option<&crate::openhuman::agent::registry::AgentRegistryEntry>,
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
    /// no chat thread bound via `thread_context::current_thread_id()`. If the
    /// agent it runs is a delegating agent (orchestrator/subconscious) and
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
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
        entry_model: Option<&str>,
    ) -> Result<Value> {
        use crate::openhuman::agent::Agent;

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
        let effective: Cow<'_, Config> = match node_model.as_deref() {
            Some(model) => {
                let mut config = (*self.config).clone();
                config.default_model = Some(harness_model_default_override(model));
                Cow::Owned(config)
            }
            None => Cow::Borrowed(self.config.as_ref()),
        };

        let mut agent =
            Agent::from_config_for_agent(effective.as_ref(), agent_ref).map_err(|e| {
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
        // `src/openhuman/security/approval/gate.rs` `Workflow { require_approval: false }`
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
        use crate::openhuman::agent::turn_origin;
        let escalated_origin = escalated_origin_for_nested_harness(turn_origin::current());
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
                agent.run_single(&prompt),
            ))
        } else {
            Box::pin(agent.run_single(&prompt))
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
    /// [`AgentRegistryEntry`](crate::openhuman::agent::registry::AgentRegistryEntry)
    /// with no harness definition — the pre-Phase-A behavior, kept so custom
    /// agents don't regress.
    async fn run_via_registry_fallback(
        &self,
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        // Resolve + validate the requested agent kind against the registry.
        let entry = crate::openhuman::agent::registry::get_agent(agent_ref)
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
            config: self.config.clone(),
        }
        .complete(request, conn)
        .await
    }
}

/// Inserts `system_prompt` as the first `system` message of a completion
/// `request`, creating the `messages` array (seeded from any `prompt` string)
/// when the request doesn't already carry one. Mirrors how
/// [`OpenHumanLlm::complete`] reads `messages`/`prompt`.
pub(crate) fn prepend_system_message(request: &mut Value, system_prompt: &str) {
    let Value::Object(map) = request else {
        return;
    };
    let system_msg = json!({ "role": "system", "content": system_prompt });
    match map.get_mut("messages").and_then(Value::as_array_mut) {
        Some(messages) => messages.insert(0, system_msg),
        None => {
            // No `messages`: build one from the `prompt` string (if any).
            let mut messages = vec![system_msg];
            if let Some(prompt) = map.get("prompt").and_then(Value::as_str) {
                messages.push(json!({ "role": "user", "content": prompt }));
            }
            map.insert("messages".to_string(), Value::Array(messages));
        }
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
