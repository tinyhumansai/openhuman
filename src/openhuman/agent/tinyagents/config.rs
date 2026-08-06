//! Maps OpenHuman's config schema into the crate-owned
//! [`tinyagents::harness::config`] structs.
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

use tinyagents::harness::config::{
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
mod tests {
    use super::*;

    fn base() -> Config {
        Config::default()
    }

    #[test]
    fn maps_the_path_roots_verbatim() {
        let c = base();
        let s = session_config_from(&c);
        assert_eq!(s.workspace_dir, c.workspace_dir);
        assert_eq!(s.action_dir, c.action_dir);
    }

    #[test]
    fn falls_back_to_default_model_when_none_is_configured() {
        let mut c = base();
        c.default_model = None;
        assert_eq!(session_config_from(&c).model, DEFAULT_MODEL);

        c.default_model = Some("some-model".into());
        assert_eq!(session_config_from(&c).model, "some-model");
    }

    #[test]
    fn turn_limits_come_from_the_agent_section() {
        let mut c = base();
        c.agent.max_tool_iterations = 7;
        c.agent.max_history_messages = 11;
        c.agent.parallel_tools = true;
        c.agent.max_parallel_tools = 3;
        c.agent.agent_timeout_secs = 45;
        // Set away from the default so a dropped assignment fails here. A
        // default-valued field is invisible to `default_config_maps_to_the_
        // crate_defaults` (it compares against `TurnConfig::default()`) and to
        // `per_section_mappers_agree_with_the_composed_one` (which compares
        // `turn_config_from` against itself), so this is the only test that can
        // catch it.
        c.agent.compact_context = !AgentConfig::default().compact_context;
        c.agent.tool_result_budget_bytes = 4242;

        let t = session_config_from(&c).turn;
        assert_eq!(t.max_tool_iterations, 7);
        assert_eq!(t.max_history_messages, 11);
        assert!(t.parallel_tools);
        assert_eq!(t.max_parallel_tools, 3);
        assert_eq!(t.timeout_secs, 45);
        assert_eq!(t.compact_context, !AgentConfig::default().compact_context);
        assert_eq!(t.tool_result_budget_bytes, 4242);
    }

    #[test]
    fn default_config_maps_to_the_crate_defaults() {
        // The two schemas drifting apart is the failure mode this whole mapper
        // exists to make visible, so pin that an unconfigured host produces an
        // unconfigured crate config.
        let s = session_config_from(&base());
        assert_eq!(s.turn, TurnConfig::default());
        assert_eq!(s.tools, ToolConfig::default());
        assert_eq!(s.memory.max_memory_context_chars, 2000);
    }

    #[test]
    fn every_dispatcher_spelling_maps_and_unknown_falls_back_to_auto() {
        for (raw, want) in [
            ("auto", ToolDispatcher::Auto),
            ("native", ToolDispatcher::Native),
            ("xml", ToolDispatcher::Xml),
            ("pformat", ToolDispatcher::Pformat),
            // Case and surrounding whitespace are tolerated.
            ("  NATIVE ", ToolDispatcher::Native),
            // A typo must not fail the session.
            ("nativ", ToolDispatcher::Auto),
            ("", ToolDispatcher::Auto),
        ] {
            assert_eq!(dispatcher_from(raw), want, "mapping {raw:?}");
        }
    }

    #[test]
    fn memory_limits_come_from_resolved_limits_not_the_legacy_field() {
        let mut c = base();
        c.agent.memory_window =
            Some(crate::openhuman::config::schema::MemoryContextWindow::Maximum);
        // The legacy scalar is deliberately set low; the preset must win.
        c.agent.max_memory_context_chars = 1;

        let want = c.agent.resolved_memory_limits();
        let got = session_config_from(&c).memory;
        assert_eq!(got.max_memory_context_chars, want.max_memory_context_chars);
        assert!(
            got.max_memory_context_chars > 1,
            "preset must override the legacy scalar"
        );
        assert_eq!(got.per_namespace_max_chars, want.per_namespace_max_chars);
        assert_eq!(got.total_tree_max_chars, want.total_tree_max_chars);
    }

    #[test]
    fn required_output_contract_is_carried_across() {
        let mut c = base();
        c.agent.required_output = Some(crate::openhuman::config::RequiredOutputContract {
            block_key: "thoughts".into(),
            required_keys: vec!["next_action".into()],
        });

        let r = session_config_from(&c)
            .turn
            .required_output
            .expect("contract is mapped");
        assert_eq!(r.block_key, "thoughts");
        assert_eq!(r.required_keys, vec!["next_action".to_string()]);
        assert!(r.is_active());
    }

    #[test]
    fn apply_agent_config_overrides_the_global_agent_section() {
        // The session builder takes a per-agent AgentConfig override. Mapping
        // only from the global Config would silently discard it and run every
        // agent on the global limits — this is the regression that guards it.
        let mut c = base();
        c.agent.max_tool_iterations = 3;

        let mut s = session_config_from(&c);
        assert_eq!(s.turn.max_tool_iterations, 3, "global applies first");

        let mut per_agent = AgentConfig::default();
        per_agent.max_tool_iterations = 25;
        per_agent.agents_md_enabled = false;
        apply_agent_config(&mut s, &per_agent);

        assert_eq!(s.turn.max_tool_iterations, 25);
        assert!(!s.agents_md_enabled);
        // Path roots and model are session-level and must survive the overlay.
        assert_eq!(s.workspace_dir, c.workspace_dir);
        assert_eq!(s.model, session_config_from(&c).model);
    }

    #[test]
    fn per_section_mappers_agree_with_the_composed_one() {
        let mut c = base();
        c.agent.max_history_messages = 9;
        c.agent.tool_dispatcher = "pformat".into();

        let s = session_config_from(&c);
        assert_eq!(s.turn, turn_config_from(&c.agent));
        assert_eq!(s.tools, tool_config_from(&c.agent));
        assert_eq!(s.memory, memory_limits_from(&c.agent));
        assert_eq!(s.tools.dispatcher, ToolDispatcher::Pformat);
    }

    #[test]
    fn required_output_from_preserves_key_semantics() {
        let host = RequiredOutputContract {
            block_key: "thoughts".into(),
            required_keys: vec!["next_action".into()],
        };
        let crate_side = required_output_from(&host);
        assert_eq!(crate_side.all_keys(), host.all_keys());
        assert_eq!(crate_side.is_active(), host.is_active());

        // The inert case must agree too — that is the one that decides whether
        // enforcement runs at all.
        let inert = RequiredOutputContract {
            block_key: "  ".into(),
            required_keys: vec!["next_action".into()],
        };
        let inert_crate = required_output_from(&inert);
        assert_eq!(inert_crate.all_keys(), inert.all_keys());
        assert!(!inert_crate.is_active());
        assert!(!inert.is_active());
    }

    #[test]
    fn base_mapping_implies_no_delegation_and_no_model_pins() {
        let s = session_config_from(&base());
        assert_eq!(s.max_depth, 0, "depth is per-delegate, not global");
        assert!(!s.may_delegate_at(0));
        assert!(s.lead_model.is_none());
        assert!(s.subagent_model.is_none());
    }

    #[test]
    fn team_pins_apply_and_a_missing_team_is_a_no_op() {
        let mut c = base();
        c.teams.insert(
            "research".into(),
            crate::openhuman::config::TeamModelConfig {
                lead_model: Some("opus".into()),
                agent_model: Some("haiku".into()),
            },
        );

        let mut s = session_config_from(&c);
        apply_team_models(&mut s, &c, "research");
        assert_eq!(s.effective_lead_model(), "opus");
        assert_eq!(s.effective_subagent_model(), "haiku");

        // An unknown team leaves the session exactly as it was.
        let mut untouched = session_config_from(&c);
        let before = untouched.clone();
        apply_team_models(&mut untouched, &c, "no-such-team");
        assert_eq!(untouched, before);
    }

    /// `TeamModelConfig`'s own doc: "Callers fall back across the pair so
    /// configs can specify only one tier without breaking routing." A team that
    /// pins one model means that model for both tiers — leaving the other on the
    /// **global** default would silently run half the team on a model the user
    /// did not choose for this team.
    #[test]
    fn a_team_pinning_only_one_tier_applies_it_to_both() {
        let mut c = base();
        c.default_model = Some("sonnet".into());
        c.teams.insert(
            "solo".into(),
            crate::openhuman::config::TeamModelConfig {
                lead_model: Some("opus".into()),
                agent_model: None,
            },
        );

        let mut s = session_config_from(&c);
        apply_team_models(&mut s, &c, "solo");
        assert_eq!(s.effective_lead_model(), "opus");
        assert_eq!(
            s.effective_subagent_model(),
            "opus",
            "a single pin covers both tiers"
        );
    }

    /// A blank pin is not a pin. Without the trim-and-drop it would replace a
    /// working default with the empty string and fail at dispatch instead.
    #[test]
    fn a_blank_team_pin_does_not_displace_the_default() {
        let mut c = base();
        c.default_model = Some("sonnet".into());
        c.teams.insert(
            "blank".into(),
            crate::openhuman::config::TeamModelConfig {
                lead_model: Some("   ".into()),
                agent_model: None,
            },
        );

        let mut s = session_config_from(&c);
        apply_team_models(&mut s, &c, "blank");
        assert_eq!(s.effective_lead_model(), "sonnet");
        assert_eq!(s.effective_subagent_model(), "sonnet");
    }

    #[test]
    fn a_blank_delegate_model_keeps_the_session_default() {
        let mut c = base();
        c.default_model = Some("sonnet".into());
        let mut s = session_config_from(&c);
        apply_delegate(
            &mut s,
            &DelegateAgentConfig {
                model: "  ".into(),
                system_prompt: None,
                temperature: None,
                max_depth: 1,
            },
        );
        assert_eq!(s.model, "sonnet");
    }

    #[test]
    fn delegate_overrides_replace_the_base_model_and_enable_delegation() {
        let c = base();
        let mut s = session_config_from(&c);
        apply_delegate(
            &mut s,
            &DelegateAgentConfig {
                model: "haiku".into(),
                system_prompt: None,
                temperature: Some(0.1),
                max_depth: 2,
            },
        );

        assert_eq!(s.model, "haiku");
        assert_eq!(s.temperature, Some(0.1));
        assert_eq!(s.max_depth, 2);
        assert!(s.may_delegate_at(1));
        assert!(!s.may_delegate_at(2));
    }

    #[test]
    fn a_delegate_without_a_temperature_keeps_the_configured_default() {
        let mut c = base();
        c.default_temperature = 0.7;
        let mut s = session_config_from(&c);
        apply_delegate(
            &mut s,
            &DelegateAgentConfig {
                model: "haiku".into(),
                system_prompt: None,
                temperature: None,
                max_depth: 1,
            },
        );
        assert_eq!(s.temperature, Some(0.7));
    }
}
