//! Maps OpenHuman's config schema into the crate-owned
//! [`tinyagents_harness::config`] structs.
//!
//! This is the host half of `docs/specs/plan-agents.md` Phase 3. The agent
//! runtime is being made generic over its host, so it cannot read
//! [`Config`] — instead the crate declares what it needs and this module is the
//! single place OpenHuman's schema meets it. Mirrors the established
//! `tinycortex::config::memory_config_from` precedent.
//!
//! # Why the mapping is split into three functions
//!
//! OpenHuman's model pins are not global. `Config::teams` is keyed by team name
//! and `Config::agents` by delegate id, so "the model for this session" is only
//! knowable once you know *which* agent is running. Folding all of that into
//! one `session_config_from(&Config)` would force it to invent an answer.
//! Instead [`session_config_from`] maps what is genuinely global, and the two
//! `apply_*` functions layer the narrower pins on top in the order the runtime
//! resolves them: team pins first, then the specific delegate's overrides.

use tinyagents_harness::config::{
    MemoryLimits, RequiredOutput, SessionConfig, ToolConfig, ToolDispatcher, TurnConfig,
};

use crate::openhuman::config::{
    AgentConfig, Config, DelegateAgentConfig, RequiredOutputContract, DEFAULT_MODEL,
};

/// Translates OpenHuman's free-form `agent.tool_dispatcher` string into the
/// crate enum.
///
/// Unknown values fall back to [`ToolDispatcher::Auto`] with a warning rather
/// than failing the session. The host schema types this field as a `String`, so
/// a typo reaches us as data that already passed config validation — refusing
/// to build a session over it would turn a cosmetic config error into an agent
/// that cannot run at all. `auto` is also what the host itself defaults to, so
/// the fallback is the documented behaviour rather than a guess.
fn dispatcher_from(raw: &str) -> ToolDispatcher {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" => ToolDispatcher::Auto,
        "native" => ToolDispatcher::Native,
        "xml" => ToolDispatcher::Xml,
        "pformat" => ToolDispatcher::Pformat,
        other => {
            tracing::warn!(
                target: "tinyagents",
                dispatcher = %other,
                "[tinyagents] unknown agent.tool_dispatcher; falling back to auto"
            );
            ToolDispatcher::Auto
        }
    }
}

/// Builds the globally-applicable [`SessionConfig`] from `config`.
///
/// Leaves [`SessionConfig::lead_model`] and [`SessionConfig::subagent_model`]
/// unset — those are per-team pins; see [`apply_team_models`]. `max_depth` is
/// left at `0` (delegation disabled) because depth is a per-delegate setting;
/// see [`apply_delegate`]. A caller that wants the plain single-agent case gets
/// exactly that, with no delegation implied by accident.
pub fn session_config_from(config: &Config) -> SessionConfig {
    let mut session = SessionConfig::new(
        config.workspace_dir.clone(),
        config.action_dir.clone(),
        config
            .default_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
    );

    session.temperature = Some(config.default_temperature);
    apply_agent_config(&mut session, &config.agent);
    session
}

/// Overlays one [`AgentConfig`] onto `session`, replacing its turn, tool, and
/// memory sections and the `agents_md_enabled` flag.
///
/// Split out from [`session_config_from`] because a session's `AgentConfig` is
/// **not always `config.agent`** — the session builder takes a per-agent
/// override. Mapping only from the global `Config` would silently discard that
/// override and run every agent on the global limits.
pub fn apply_agent_config(session: &mut SessionConfig, agent: &AgentConfig) {
    session.agents_md_enabled = agent.agents_md_enabled;
    session.turn = turn_config_from(agent);
    session.tools = tool_config_from(agent);
    session.memory = memory_limits_from(agent);
}

/// Maps the per-turn limits out of an [`AgentConfig`].
pub fn turn_config_from(agent: &AgentConfig) -> TurnConfig {
    TurnConfig {
        max_tool_iterations: agent.max_tool_iterations,
        max_history_messages: agent.max_history_messages,
        compact_context: agent.compact_context,
        parallel_tools: agent.parallel_tools,
        max_parallel_tools: agent.max_parallel_tools,
        tool_result_budget_bytes: agent.tool_result_budget_bytes,
        timeout_secs: agent.agent_timeout_secs,
        required_output: agent.required_output.as_ref().map(required_output_from),
    }
}

/// Maps tool dispatch and reachability out of an [`AgentConfig`].
pub fn tool_config_from(agent: &AgentConfig) -> ToolConfig {
    ToolConfig {
        dispatcher: dispatcher_from(&agent.tool_dispatcher),
        channel_permissions: agent.channel_permissions.clone(),
    }
}

/// Maps memory character budgets out of an [`AgentConfig`].
///
/// Reads through `resolved_memory_limits()` rather than the legacy
/// `max_memory_context_chars` scalar: that helper is what applies the
/// `memory_window` preset and the hard ceiling, and bypassing it drops both.
pub fn memory_limits_from(agent: &AgentConfig) -> MemoryLimits {
    let limits = agent.resolved_memory_limits();
    MemoryLimits {
        max_memory_context_chars: limits.max_memory_context_chars,
        per_namespace_max_chars: limits.per_namespace_max_chars,
        total_tree_max_chars: limits.total_tree_max_chars,
    }
}

/// Converts the host's structured-output contract into the crate's.
///
/// The two types are field-identical by design; this is the one place that
/// equivalence is asserted, so a divergence shows up here rather than as a
/// silently unenforced contract.
pub fn required_output_from(contract: &RequiredOutputContract) -> RequiredOutput {
    RequiredOutput {
        block_key: contract.block_key.clone(),
        required_keys: contract.required_keys.clone(),
    }
}

/// Applies the `[teams.<team>]` model pins to an already-mapped `session`.
///
/// A missing team is not an error — it means no pin, so the session keeps the
/// global default model.
pub fn apply_team_models(session: &mut SessionConfig, config: &Config, team: &str) {
    let Some(pins) = config.teams.get(team) else {
        tracing::debug!(
            target: "tinyagents",
            %team,
            "[tinyagents] no team model pins; keeping the global default model"
        );
        return;
    };
    // Resolve through `model_for_role` rather than copying the raw options.
    // That helper owns three behaviours this mapper must not re-derive: it
    // trims, it drops empty strings (so a blank pin cannot displace a valid
    // default), and it falls back across the pair — a team that sets only
    // `lead_model` means that model for *both* tiers. Copying the fields
    // directly left the unset tier on the global default, which is a different
    // model from the one the user configured.
    if let Some(lead) = pins.model_for_role(true) {
        session.lead_model = Some(lead.to_string());
    }
    if let Some(agent) = pins.model_for_role(false) {
        session.subagent_model = Some(agent.to_string());
    }
}

/// Applies one delegate agent's overrides — its model, temperature, and the
/// nesting depth it is permitted.
///
/// `delegate.model` overwrites [`SessionConfig::model`] rather than
/// `lead_model`: a delegate *is* the agent running this session, so it is the
/// base model, not an override layered over some other base.
pub fn apply_delegate(session: &mut SessionConfig, delegate: &DelegateAgentConfig) {
    // `DelegateAgentConfig::model` is a bare `String`, and TOML happily accepts
    // `model = ""`. Assigning that unchecked would replace a working session
    // model with the empty string and fail at dispatch rather than here, so a
    // blank pin leaves the session default alone — the same trim-and-drop rule
    // `TeamModelConfig::model_for_role` applies to team pins.
    let model = delegate.model.trim();
    if model.is_empty() {
        tracing::warn!(
            target: "tinyagents",
            "[tinyagents] delegate model pin is blank; keeping the session default model"
        );
    } else {
        session.model = model.to_string();
    }
    if let Some(t) = delegate.temperature {
        session.temperature = Some(t);
    }
    session.max_depth = delegate.max_depth;
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
