//! Where an embedded agent's own skills live.
//!
//! An agent derived on a shared workspace keeps the skills it installs or
//! creates under `<workspace>/agents/<id>/`, in place of the operator's home
//! directory: `skills/` for SKILL.md bundles, `workflows/` for automations.
//! Discovery scans those roots for that agent only, so a skill one agent
//! writes is neither seen by its siblings nor written into the operator's
//! `~/.openhuman`.
//!
//! A SaaS profile's default agent has no agent id, but it must not reach the
//! operator's home either: its skills home is
//! `<workspace>/agents/default/` ([`DEFAULT_AGENT`]) under the profile's own
//! workspace, keyed by the tenant rather than the agent. A SaaS task with no
//! tenant scope gets no home and no install root at all.
//!
//! [`DEFAULT_AGENT`]: crate::storage::sessions::DEFAULT_AGENT

use std::path::{Path, PathBuf};

use crate::core::runtime::Tenant;

/// The current tenant's skills home under `workspace_dir`: the embedded
/// agent's, or a SaaS profile's default agent's. `None` for the desktop's own
/// default orchestrator (which uses the operator's home) and for a SaaS task
/// with no scope.
#[must_use]
pub fn agent_skill_home(workspace_dir: &Path) -> Option<PathBuf> {
    let tenant = crate::core::runtime::DispatchCtx::ambient().tenant().ok()?;
    skill_home_for(&tenant, workspace_dir)
}

/// [`agent_skill_home`] for an explicit tenant.
#[must_use]
pub fn skill_home_for(tenant: &Tenant, workspace_dir: &Path) -> Option<PathBuf> {
    let agent = match (&tenant.agent, &tenant.profile) {
        (Some(agent), _) => agent.as_str(),
        (None, Some(_)) => crate::storage::sessions::DEFAULT_AGENT,
        (None, None) => return None,
    };
    Some(workspace_dir.join("agents").join(agent))
}

/// The current tenant's skills home under its context's workspace, or `None`
/// where [`agent_skill_home`] has none — for callers that hold no workspace
/// path.
#[must_use]
pub fn current_agent_skill_home() -> Option<PathBuf> {
    let saas = crate::core::runtime::is_saas();
    let workspace = crate::core::runtime::tenant::context_in(saas)?
        .workspace_dir()
        .ok()?;
    agent_skill_home(&workspace)
}

/// Whether a caller with no tenant skills home may fall back to the
/// operator's `~/.openhuman`: never in SaaS, where that home is shared by
/// every user.
fn operator_home_allowed() -> bool {
    if crate::core::runtime::is_saas() {
        log::warn!("[skills] no tenant skills home in SaaS; refusing the operator's home");
        return false;
    }
    true
}

/// The roots an agent's user-scope bundles are discovered from and removed
/// from, newest layout last so it wins a name collision.
#[must_use]
pub fn agent_user_roots(agent_home: &Path) -> [PathBuf; 2] {
    [agent_home.join("skills"), agent_home.join("workflows")]
}

/// Where a user-scope SKILL.md bundle is installed: the agent's `skills/`
/// under an embedded agent's context, else `~/.openhuman/skills`.
#[must_use]
pub fn user_skill_install_root(workspace_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    match agent_skill_home(workspace_dir) {
        Some(agent_home) => Some(agent_home.join("skills")),
        None if !operator_home_allowed() => None,
        None => home.map(|home| home.join(".openhuman").join("skills")),
    }
}

/// Where a user-scope workflow is created: the agent's `workflows/` under an
/// embedded agent's or a SaaS profile's context, else
/// `~/.openhuman/workflows` (never in SaaS).
#[must_use]
pub fn user_workflow_root(workspace_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    match agent_skill_home(workspace_dir) {
        Some(agent_home) => Some(agent_home.join("workflows")),
        None if !operator_home_allowed() => None,
        None => home.map(|home| home.join(".openhuman").join("workflows")),
    }
}

#[cfg(test)]
#[path = "write_root_tests.rs"]
mod tests;
