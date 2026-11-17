//! `OpenHumanSessionHost::from_config` factory methods and internal `build_session_agent_inner` constructor.

use super::should_synthesize_delegation_tools;
use crate::agent::prompts::SystemPromptBuilder;
use crate::agent::session_host::types::OpenHumanSessionHost;
use crate::agent_registry::host_definition::{
    AgentDefinitionExt, AgentDefinitionRegistry, NO_TOOLS_SENTINEL,
};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::modules::tinybox::host_runtime;
use crate::security::SecurityPolicy;
use crate::tools;
use anyhow::Result;
use std::sync::Arc;
use tinyagents_definition::{PromptSource, ToolScope};
use tinytools::Tool;
use tinytools_agent::dialect::{
    CodeDialect, CodeStyle, NativeDialect, PFormatDialect, ToolCallFormat, ToolDialect, XmlDialect,
};

#[path = "factory/build_session.rs"]
mod build_session;
#[path = "factory_tool_registry.rs"]
mod tool_registry;
#[path = "factory_workflows.rs"]
mod workflows;

impl OpenHumanSessionHost {
    /// Returns whether `agent_id` resolves to a runnable definition for this
    /// configuration. This is deliberately the same resolution path used by
    /// [`Self::from_config_for_agent`], so configuration writers cannot save
    /// a web-chat route that the session factory would later reject.
    pub(crate) fn is_runnable_agent_id(config: &Config, agent_id: &str) -> bool {
        resolve_target_definition(config, agent_id).is_ok()
    }

    /// Constructs an `OpenHumanSessionHost` instance from a global system configuration.
    ///
    /// Thin wrapper around [`OpenHumanSessionHost::from_config_for_agent`] targeting the
    /// orchestrator for legacy CLI / REPL callers.
    /// that does not participate in the #525 onboarding-routing flow.
    ///
    /// Callers that need to select a different agent at session-build
    /// time (for example the Tauri web chat path, which routes to the
    /// welcome agent pre-onboarding) should call
    /// [`OpenHumanSessionHost::from_config_for_agent`] directly.
    pub async fn from_config(ctx: &DispatchCtx, config: &Config) -> Result<Self> {
        Self::from_config_for_agent(ctx, config, "orchestrator").await
    }

    /// Constructs an `OpenHumanSessionHost` instance scoped to a specific agent
    /// definition loaded from the global [`AgentDefinitionRegistry`].
    ///
    /// `agent_id` is looked up in the registry; the returned agent
    /// inherits that definition's `ToolScope`, `system_prompt`,
    /// `temperature`, `max_iterations`, and `omit_*` flags. Unknown
    /// agent ids produce a registry-lookup error rather than silently
    /// falling back to the orchestrator.
    ///
    /// Shared infrastructure between agent ids is identical:
    /// 1. Initializing the host runtime (native or docker).
    /// 2. Setting up security policies.
    /// 3. Registering all built-in and orchestrator tools (the `memory` tool
    ///    only while memory is on).
    /// 4. Configuring the routed AI provider.
    /// 5. Setting up post-turn hooks.
    ///
    /// What differs per agent id:
    /// * `visible_tool_names` is the agent's `ToolScope::Named` list
    ///   (unioned with the names of synthesised delegation tools when
    ///   the agent declares `[subagents] allowlist = [...]`). `ToolScope::Wildcard`
    ///   yields an empty filter, matching the legacy unfiltered path.
    /// * `prompt_builder` uses [`SystemPromptBuilder::for_subagent`]
    ///   with the agent's inline/file prompt body and `omit_*` flags,
    ///   so each agent renders its own persona rather than the default
    ///   orchestrator workspace-files identity dump.
    /// * `temperature` comes from the agent's TOML (falls back to
    ///   `config.default_temperature` for the orchestrator to preserve
    ///   legacy behaviour).
    ///
    /// The welcome agent uses this entry point when routed from the
    /// Tauri web channel (see `channels::provider::web::build_session_agent`).
    pub async fn from_config_for_agent(
        ctx: &DispatchCtx,
        config: &Config,
        agent_id: &str,
    ) -> Result<Self> {
        let target_def = resolve_target_definition(config, agent_id)?;

        log::info!(
            "[agent::builder] building session agent id={} \
             (scope={}, omit_identity={}, omit_memory_context={}, temperature={:.2})",
            agent_id,
            target_def
                .as_ref()
                .map(|d| match &d.tools {
                    ToolScope::Named(names) => format!("named({})", names.len()),
                    ToolScope::Wildcard => "wildcard".to_string(),
                })
                .unwrap_or_else(|| "legacy".to_string()),
            target_def
                .as_ref()
                .is_some_and(|d| d.extensions.omit_identity),
            target_def
                .as_ref()
                .is_some_and(|d| d.extensions.omit_memory_context),
            target_def
                .as_ref()
                .map(|d| d.temperature)
                .unwrap_or(config.default_temperature)
        );

        Self::build_session_agent_inner(
            ctx,
            config,
            agent_id,
            target_def.as_ref(),
            false,
            None,
            None,
        )
        .await
    }

    /// Internal constructor that consumes the optionally-resolved agent
    /// definition. Split out from [`OpenHumanSessionHost::from_config_for_agent`] so
    /// the lookup + logging live in one place and the heavy-lifting
    /// body stays readable.
    // `pub(crate)` (rather than private) so `builder_tests` can drive the
    // definition-cap resolution logic (issue #4868) directly with a
    // hand-picked `target_def`, independent of the process-global
    // `AgentDefinitionRegistry` singleton's init-once state. Still not part
    // of the crate's public API.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn build_session_agent_inner(
        ctx: &DispatchCtx,
        config: &Config,
        agent_id: &str,
        target_def: Option<&crate::agent_registry::host_definition::AgentDefinition>,
        host_only: bool,
        host: Option<&super::HostTools>,
        session_id: Option<&str>,
    ) -> Result<Self> {
        Self::build_session_agent_inner_with_turn_model_source(
            ctx, config, agent_id, target_def, host_only, host, session_id, None,
        )
        .await
    }
}

/// Resolves the `AgentDefinition` a session should be built from, given the
/// requested `agent_id`, in three steps:
///
/// 1. **Harness registry** (`AgentDefinitionRegistry`, the process-global
///    singleton of built-in + workspace-TOML-override definitions) — a hit
///    here wins outright.
/// 2. **Config-backed custom agent registry** (`config.agent_registry.entries`,
///    `AgentRegistrySource::Custom`) — on a harness-registry miss (or the
///    registry not yet being initialised), a user-authored custom agent is
///    synthesized into a real `AgentDefinition` via
///    `agent_registry::definition_from_registry_entry` so it runs through
///    the exact same `build_session_agent_inner` path (and therefore the
///    exact same `SecurityPolicy` / tool-filtering / approval gate) as a
///    built-in. This closes the gap where a custom agent either hard-errored
///    (chat) or silently ran tool-less/persona-only (flows'
///    `RegistryFallback`) — see the cross-cutting fix in the PR that added
///    this function.
/// 3. **Orchestrator legacy fallback** — `orchestrator` alone is allowed to
///    resolve to `None` (pre-startup, tests): the caller then builds with the
///    default prompt/filter, matching pre-#1 behaviour.
///
/// Any other id that resolves nowhere is a hard error, exactly as before this
/// function existed — only the *search order* changed, not the failure
/// contract for a genuinely-unknown id.
pub(super) fn resolve_target_definition(
    config: &Config,
    agent_id: &str,
) -> Result<Option<crate::agent_registry::host_definition::AgentDefinition>> {
    let registry = AgentDefinitionRegistry::current();

    if let Some(reg) = registry.as_deref() {
        if let Some(def) = reg.get(agent_id) {
            return Ok(Some(def.clone()));
        }
    }

    // Harness registry miss (or not yet initialised). Before failing, check
    // the config-backed custom agent registry — the one place custom
    // (non-shipped) agents live.
    if let Some(entry) = crate::agent_registry::find_custom_in_config(config, agent_id) {
        log::info!(
            "[agent::builder] agent_id={} not found in the harness AgentDefinitionRegistry — \
             synthesizing a definition from its custom agent_registry entry so it runs with its \
             real tool belt instead of persona-only / erroring",
            agent_id
        );
        return Ok(Some(crate::agent_registry::definition_from_registry_entry(
            &entry,
        )));
    }

    if agent_id == "orchestrator" {
        // Orchestrator is allowed to be missing from every source (legacy
        // path, tests, pre-startup) — fall back to default behaviour.
        log::debug!(
            "[agent::builder] orchestrator definition not in any registry — using legacy \
             default prompt + filter"
        );
        return Ok(None);
    }

    if registry.is_none() {
        return Err(anyhow::anyhow!(
            "AgentDefinitionRegistry is not initialised — cannot resolve agent '{}'. Call \
             AgentDefinitionRegistry::init_global at startup.",
            agent_id
        ));
    }

    Err(anyhow::anyhow!(
        "agent definition '{}' not found in registry",
        agent_id
    ))
}
