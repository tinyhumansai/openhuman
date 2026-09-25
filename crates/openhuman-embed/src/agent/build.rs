//! Turning an [`AgentSpec`] into an [`AgentInner`] on a [`Runtime`].
//!
//! Order matters and is fixed here: validate the id, lay out directories,
//! assemble the per-agent `Config` (base → access → provider → MCP →
//! Composio → escape hatch), build the definition, copy skills, check the
//! narrowing rules, derive the context.

use std::path::Path;

use openhuman_core::core::all::DomainGroup;
use openhuman_core::core::runtime::{ContextOverlay, DomainSet};
use openhuman_core::tools::toolpacks::{GroupMode, ToolGroups};

use super::{AgentError, AgentInner, AgentLayout, AgentSpec};
use crate::runtime::Runtime;

pub(crate) fn instantiate(runtime: &Runtime, spec: AgentSpec) -> Result<AgentInner, AgentError> {
    let parts = spec.into_parts();
    let id = parts.id;
    validate_agent_id(&id).map_err(|reason| AgentError::InvalidId {
        id: id.clone(),
        reason,
    })?;

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

    let mut access = parts
        .access
        .unwrap_or_else(|| runtime.default_access().clone());
    for (path, grant) in parts.trusted {
        access = access.trust(path, grant);
    }
    access.apply(&mut config);

    let provider = parts
        .provider
        .unwrap_or_else(|| runtime.default_provider().clone());
    crate::runtime::builder::apply_provider(&mut config, &provider);

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

    if let Some(f) = parts.config_fn {
        f(&mut config);
        // Every agent shares the runtime's credential store and keyring; a
        // moved `config_path` would silently point this agent at another.
        config.config_path = base.config_path.clone();
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
    let definition = parts.definition.into_core(&id)?;

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
    let overlay = ContextOverlay {
        config: config.clone(),
        domains,
        tool_groups,
        user_skill_roots: parts.include_user_skills,
    };
    let ctx = runtime.core_runtime().context().derive_with(overlay);

    log::debug!(
        "[embed][agent] instantiated id={id} action_dir={} routed={} access_origin={} \
         user_skills={}",
        config.action_dir.display(),
        provider.is_routed(),
        access.turn_origin().is_some(),
        parts.include_user_skills
    );

    Ok(AgentInner {
        id,
        _runtime_guard: runtime.guard(),
        runtime: runtime.core_runtime().clone(),
        ctx,
        config,
        definition,
        provider,
        access,
        layout,
        host_tools: parts.host_tools,
    })
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
