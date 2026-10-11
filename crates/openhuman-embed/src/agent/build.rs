//! Turning an [`AgentSpec`] into an [`AgentInner`] on a [`Runtime`].
//!
//! Order matters and is fixed here: validate the id, lay out directories,
//! assemble the per-agent `Config` (base → access → provider → MCP →
//! Composio → memory binding → escape hatch), build the definition, copy skills, check the
//! narrowing rules, derive the context.

use std::path::Path;

use openhuman_core::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, SubagentEntry,
};
use openhuman_core::core::all::DomainGroup;
use openhuman_core::core::runtime::{ContextOverlay, DomainSet};
use openhuman_core::tools::toolpacks::{GroupMode, ToolGroups};

use super::{AgentError, AgentInner, AgentLayout, AgentSpec};
use crate::harness::Access;
use crate::runtime::Runtime;

pub(crate) fn instantiate(runtime: &Runtime, spec: AgentSpec) -> Result<AgentInner, AgentError> {
    let mut parts = spec.into_parts();
    let defaults = runtime.defaults();
    let mut runtime_definition = defaults.definition.clone();
    if defaults.sandbox != crate::SandboxModeSpec::None {
        runtime_definition = runtime_definition.sandbox(defaults.sandbox);
    }
    if let Some(v) = defaults.model.temperature {
        runtime_definition = runtime_definition.temperature(v);
    }
    if let Some(v) = defaults.model.max_iterations {
        runtime_definition = runtime_definition.max_iterations(v);
    }
    let mut inherited = if let Some(name) = parts.template.take() {
        defaults
            .templates
            .get(&name)
            .cloned()
            .ok_or(AgentError::UnknownTemplate(name))?
            .inherit(runtime_definition)?
    } else {
        runtime_definition
    };
    let mut model_defaults = defaults.model.overlay(&parts.model_defaults);
    model_defaults.validate().map_err(AgentError::Invalid)?;
    if let Some(v) = parts.model_defaults.temperature {
        inherited = inherited.temperature(v);
    }
    if let Some(v) = parts.model_defaults.max_iterations {
        inherited = inherited.max_iterations(v);
    }
    parts.definition = parts.definition.inherit(inherited)?;
    let include_user_skills = parts
        .include_user_skills
        .unwrap_or(defaults.skills.include_user_skills);
    #[cfg(feature = "skills")]
    {
        parts.skills_dir = parts.skills_dir.or(defaults.skills.root.clone());
    }
    #[cfg(feature = "mcp")]
    {
        let mut servers = defaults.mcp_baseline.clone();
        servers.extend(parts.mcp_servers);
        parts.mcp_servers = servers;
    }

    let id = parts.id;
    validate_agent_id(&id).map_err(|reason| AgentError::InvalidId {
        id: id.clone(),
        reason,
    })?;
    let catalogue = process_catalogue();
    if catalogue.get(&id).is_some() {
        return Err(AgentError::ReservedId(id));
    }

    // A host-only agent must never act, whatever else the spec says, so its
    // exclusions are settled before any of the spec is applied.
    let host_only = parts.definition.is_host_only();
    if host_only {
        #[cfg(feature = "mcp")]
        if !parts.mcp_servers.is_empty() {
            return Err(AgentError::Invalid(
                "a HostOnly agent cannot declare MCP servers; supply its tools as host tools"
                    .into(),
            ));
        }
        #[cfg(feature = "skills")]
        if parts.skills_dir.is_some() {
            return Err(AgentError::Invalid(
                "a HostOnly agent cannot install skills".into(),
            ));
        }
    }

    let base = runtime.base_config();
    let root_dir = base
        .config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    // ── config ───────────────────────────────────────────────────────────
    let mut config = base.clone();
    // Modeled default; `config.action_dir` is the field every later step
    // (including the `config_fn` escape hatch below) actually reads and can
    // override, so the directory we create and the layout we resolve are
    // taken from it again below, after every step has had a chance to touch
    // it — not from this local, which would go stale the moment a caller's
    // `config_fn` edits `config.action_dir`.
    config.action_dir = parts.action_dir.unwrap_or_else(|| {
        AgentLayout::default_action_dir(
            &root_dir,
            &base.action_dir,
            runtime.inherited_workspace(),
            &id,
        )
    });

    let mut access = parts.access.unwrap_or_else(|| defaults.access.clone());
    for (path, grant) in parts.trusted {
        access = access.trust(path, grant);
    }
    if host_only {
        // Read-only regardless of what was asked: no tier that could park a
        // write for approval, no automation origin, no trusted roots.
        log::debug!("[embed][agent] id={id} is host-only; forcing read-only access");
        access = Access::readonly();
    }
    access.apply(&mut config);

    let provider = if parts.inherit_provider_route {
        match parts.provider.as_ref().and_then(crate::Provider::model_id) {
            Some(model) => defaults.provider.clone().model(model),
            None => defaults.provider.clone(),
        }
    } else {
        parts.provider.unwrap_or_else(|| defaults.provider.clone())
    };
    crate::runtime::builder::apply_provider(&mut config, &provider);
    if let Some(v) = parts.model_defaults.temperature {
        config.default_temperature = v;
    }
    if let Some(v) = parts.model_defaults.max_iterations {
        config.agent.max_tool_iterations_override = Some(v);
    }

    #[cfg(feature = "mcp")]
    if !parts.mcp_servers.is_empty() {
        config.mcp_client.enabled = true;
        config.mcp_client.servers.extend(
            parts
                .mcp_servers
                .iter()
                .cloned()
                .map(crate::harness::McpServer::into_config),
        );
    }

    if let Some(credential) = parts.composio {
        log::debug!("[embed][agent] id={id} pins its own composio credential");
        config.composio.pin_host_credential(credential);
    }

    if let Some(binding) = parts.memory {
        let agent_id = binding.agent_id().trim();
        if agent_id.is_empty() {
            return Err(AgentError::Invalid(
                "a memory binding needs an agent id".into(),
            ));
        }
        if let Some(root) = binding.root_namespace() {
            openhuman_core::memory::scope::validate_root(root)
                .map_err(|reason| AgentError::Invalid(format!("memory root {root:?}: {reason}")))?;
        }
        log::debug!("[embed][agent] id={id} binds its memory agent_id={agent_id}");
        config.memory.agent_id = Some(agent_id.to_string());
        config.memory.root = binding.root_namespace().map(|root| root.trim().to_string());
    }

    if let Some(f) = parts.config_fn {
        f(&mut config);
        // Every agent shares the runtime's credential store and keyring; a
        // moved `config_path` would silently point this agent at another.
        config.config_path = base.config_path.clone();
    }
    if host_only {
        // After the escape hatch, so it cannot loosen either.
        Access::readonly().apply(&mut config);
        config.mcp_client.enabled = false;
    }

    // Read back now, after `config_fn` (the escape hatch, applied above) has
    // had its chance to edit `config.action_dir` — the directory created and
    // the layout resolved below must match whatever it ends up being, not
    // the pre-`config_fn` default computed further up.
    let action_dir = config.action_dir.clone();
    std::fs::create_dir_all(&action_dir).map_err(|source| AgentError::Workspace {
        what: "create the agent's action directory",
        source,
    })?;
    let layout = AgentLayout::resolve(&config.workspace_dir, &id, action_dir);
    std::fs::create_dir_all(&layout.home).map_err(|source| AgentError::Workspace {
        what: "create the agent's home",
        source,
    })?;
    std::fs::create_dir_all(&layout.skills).map_err(|source| AgentError::Workspace {
        what: "create the agent's skills directory",
        source,
    })?;

    // ── skills ───────────────────────────────────────────────────────────
    #[cfg(feature = "skills")]
    if let Some(dir) = parts.skills_dir.as_deref() {
        let dest = match parts.skills_dest {
            super::spec::SkillsDest::AgentLocal => layout.skills.clone(),
            super::spec::SkillsDest::WorkspaceLegacy => config.workspace_dir.join("skills"),
        };
        crate::harness::skills::install(dir, &dest).map_err(map_harness_err)?;
    }

    // ── definition ───────────────────────────────────────────────────────
    let mut definition = parts.definition.into_core(&id)?;
    model_defaults.temperature = Some(definition.temperature);
    model_defaults.max_iterations = Some(definition.max_iterations);
    if config.agent.max_tool_iterations_override.is_some() {
        config.agent.max_tool_iterations_override = Some(definition.max_iterations);
    }

    // Every declared server's tools are registered as their own
    // `mcp_<server>_<tool>`, deferred by default. A wildcard belt reaches them
    // through `tool_search` already; a named belt reaches deferred tools only
    // when it lists `tool_search`, so declaring a server implies it.
    #[cfg(feature = "mcp")]
    if !parts.mcp_servers.is_empty() {
        opt_named_belt_into_discovery(&mut definition.tools);
    }

    let definitions = own_catalogue(&catalogue, &id, &mut definition, parts.subagents)?;

    // ── narrowing ────────────────────────────────────────────────────────
    let domains = match parts.domains {
        Some(requested) => {
            check_domains_narrow(requested, runtime.domains())?;
            requested
        }
        None => runtime.domains(),
    };
    let tool_groups = match parts.tool_groups {
        Some(requested) => {
            check_tool_groups_narrow(&requested, runtime.tool_groups())?;
            requested
        }
        None => runtime.tool_groups().clone(),
    };

    // ── context ──────────────────────────────────────────────────────────
    let mut context_config = config.clone();
    context_config.ephemeral_route = provider.route().and_then(|route| {
        openhuman_core::config::schema::EphemeralRoute::from_params(
            Some(route.base_url.clone()),
            Some(route.api_key.clone()),
        )
        .map(|scoped| scoped.with_headers(route.headers.clone()))
    });
    let mut overrides = openhuman_core::agent::host_overrides::HostOverrides::default();
    overrides.parent = runtime.core_runtime().context().host_overrides();
    let lifecycle = super::lifecycle::Lifecycle::new();
    overrides.approval_scope = Some(lifecycle.approval_scope());
    overrides.model = provider.custom_model();
    overrides.role_models = provider.role_models().clone();
    overrides.session_store = parts.session_store;
    let overrides = std::sync::Arc::new(overrides);
    for hook in parts.post_turn_hooks {
        overrides.post_turn_hook(hook.name(), Some(hook.clone()));
    }
    for hook in parts.tool_hooks {
        overrides.tool_hook(hook.name(), Some(hook.clone()));
    }
    let approval_subscription = parts.approval_handler.map(|handler| {
        super::approval_handler::ApprovalSubscription::new(
            &id,
            handler,
            lifecycle.removed(),
            lifecycle.approval_state(),
        )
    });
    let overlay = ContextOverlay {
        host_overrides: Some(overrides.clone()),
        config: context_config,
        domains,
        tool_groups,
        user_skill_roots: include_user_skills,
        // A host session store keeps each agent's conversations apart by id.
        session_agent: Some(id.to_string()),
        profile: None,
        agent_policy: Some(std::sync::Arc::new(
            openhuman_core::security::SecurityPolicy::from_config(
                &config.autonomy,
                &config.workspace_dir,
                &config.action_dir,
            )
            .with_privacy_mode(config.privacy.mode),
        )),
        approvals_disabled: !access.approval_gate_enabled(),
        definitions,
    };
    let ctx = runtime.core_runtime().context().derive_with(overlay);
    openhuman_core::core::runtime::AgentContextRegistry::register(&id, &ctx);

    log::debug!(
        "[embed][agent] instantiated id={id} action_dir={} routed={} access_origin={} \
         user_skills={} tier={:?} approval_gate={}",
        config.action_dir.display(),
        provider.is_routed(),
        access.turn_origin().is_some(),
        include_user_skills,
        config.autonomy.level,
        access.approval_gate_enabled()
    );

    Ok(AgentInner {
        registry: runtime.agent_registry(),
        overrides,
        _approval_subscription: approval_subscription,
        id,
        runtime_id: runtime.runtime_id().to_owned(),
        attachments: Default::default(),
        _runtime_guard: runtime.guard(),
        runtime: runtime.core_runtime().clone(),
        ctx,
        config,
        definition,
        provider,
        model_defaults,
        access,
        layout,
        host_tools: parts.host_tools,
        hooks: parts.hooks,
        lifecycle,
        host_only,
    })
}

/// The process catalogue: the booted registry, or the built-ins when none
/// was initialised.
fn process_catalogue() -> std::sync::Arc<AgentDefinitionRegistry> {
    AgentDefinitionRegistry::global_arc()
        .unwrap_or_else(|| std::sync::Arc::new(AgentDefinitionRegistry::builtins_only()))
}

/// The catalogue an agent with its own sub-agents delegates through: the
/// process catalogue plus the agent and its sub-agents. `None` (resolve
/// through the process catalogue) for an agent without sub-agents.
fn own_catalogue(
    catalogue: &AgentDefinitionRegistry,
    id: &str,
    definition: &mut AgentDefinition,
    subagents: Vec<(String, super::AgentDefinitionSpec)>,
) -> Result<Option<std::sync::Arc<AgentDefinitionRegistry>>, AgentError> {
    if subagents.is_empty() {
        return Ok(None);
    }
    let mut own = Vec::with_capacity(subagents.len() + 1);
    let mut seen = std::collections::HashSet::new();
    for (sub_id, spec) in subagents {
        validate_agent_id(&sub_id).map_err(|reason| AgentError::InvalidId {
            id: sub_id.clone(),
            reason,
        })?;
        if catalogue.get(&sub_id).is_some() {
            return Err(AgentError::ReservedId(sub_id));
        }
        if sub_id == id || !seen.insert(sub_id.clone()) {
            return Err(AgentError::DuplicateId(sub_id));
        }
        definition
            .subagents
            .push(SubagentEntry::AgentId(sub_id.clone()));
        own.push(spec.into_subagent_core(&sub_id)?);
    }
    own.push(definition.clone());
    log::debug!(
        "[embed][agent] id={id} owns a catalogue with {} sub-agent(s)",
        own.len() - 1
    );
    Ok(Some(std::sync::Arc::new(catalogue.with_definitions(own))))
}

/// Adds `tool_search` to a named belt that lacks it. A wildcard belt already
/// has discovery.
#[cfg(feature = "mcp")]
fn opt_named_belt_into_discovery(
    scope: &mut openhuman_core::agent::harness::definition::ToolScope,
) {
    use openhuman_core::agent::harness::definition::ToolScope;
    const TOOL_SEARCH: &str = "tool_search";
    if let ToolScope::Named(names) = scope {
        if !names.iter().any(|name| name == TOOL_SEARCH) {
            names.push(TOOL_SEARCH.to_string());
            log::debug!("[embed][agent] declared MCP servers opt the named belt into tool_search");
        }
    }
}

fn validate_agent_id(id: &str) -> Result<(), String> {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err("must be 1 to 64 characters".to_string());
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return Err("must start with a lowercase letter or digit".to_string());
    }
    if bytes.iter().any(|byte| {
        !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && *byte != b'_' && *byte != b'-'
    }) {
        return Err("may contain only lowercase letters, digits, '_' and '-'".to_string());
    }
    Ok(())
}

/// Every family the agent asks for must be one the runtime registered.
pub(crate) fn check_domains_narrow(
    requested: DomainSet,
    runtime: DomainSet,
) -> Result<(), AgentError> {
    let widened: Vec<String> = DomainGroup::ALL
        .iter()
        .filter(|group| requested.allows(**group) && !runtime.allows(**group))
        .map(|group| format!("{group:?}").to_lowercase())
        .collect();
    if widened.is_empty() {
        Ok(())
    } else {
        Err(AgentError::WidensRuntime(format!(
            "domain families not registered by the runtime: {}",
            widened.join(", ")
        )))
    }
}

/// A group may only be as exposed as the runtime exposes it:
/// `Off` ⊂ `Withheld` ⊂ `Advertised`.
pub(crate) fn check_tool_groups_narrow(
    requested: &ToolGroups,
    runtime: &ToolGroups,
) -> Result<(), AgentError> {
    fn exposure(mode: GroupMode) -> u8 {
        match mode {
            GroupMode::Off => 0,
            GroupMode::Withheld => 1,
            GroupMode::Advertised => 2,
        }
    }
    let widened: Vec<&str> = ToolGroups::ids()
        .filter(|id| exposure(requested.mode(id)) > exposure(runtime.mode(id)))
        .collect();
    if widened.is_empty() {
        Ok(())
    } else {
        Err(AgentError::WidensRuntime(format!(
            "tool groups more exposed than the runtime's: {}",
            widened.join(", ")
        )))
    }
}

#[cfg(feature = "skills")]
fn map_harness_err(err: crate::HarnessError) -> AgentError {
    match err {
        crate::HarnessError::Workspace { what, source } => AgentError::Workspace { what, source },
        crate::HarnessError::Invalid(msg) => AgentError::Invalid(msg),
        crate::HarnessError::Call(e) => AgentError::Call(e),
        crate::HarnessError::Build(e) => AgentError::Invalid(format!("{e:#}")),
        crate::HarnessError::AlreadyRunning => AgentError::Invalid(err.to_string()),
    }
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
