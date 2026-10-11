//! Removing an installed user-scope skill from disk.

use std::path::Path;

use serde::{Deserialize, Serialize};

use tinyskills::SkillScope as WorkflowScope;

/// Input for [`uninstall_workflow`]. Mirrors the `skills.uninstall` JSON-RPC payload.
#[derive(Debug, Clone, Deserialize)]
pub struct UninstallWorkflowParams {
    /// On-disk slug of the installed skill — the directory name under
    /// `~/.openhuman/skills/<slug>/`. Retained as `name` for wire-format
    /// back-compat with pre-existing clients; semantics are slug-only.
    pub name: String,
}

/// Outcome of a successful uninstall.
#[derive(Debug, Clone, Serialize)]
pub struct UninstallWorkflowOutcome {
    /// The normalised slug that was removed.
    pub name: String,
    /// Absolute on-disk path that was deleted (post-canonicalisation).
    pub removed_path: String,
    /// Scope the uninstall applied to. Always `User` today.
    pub scope: WorkflowScope,
}

/// Remove an installed user-scope SKILL.md skill from `~/.openhuman/skills/`.
///
/// Only user-scope uninstalls are supported. Resolution is defensive
/// (`tinyskills::remove_bundle`): canonicalises paths, refuses symlinks,
/// requires a workflow document to be present.
///
/// `home_dir_override` is for tests; production callers pass `None`.
pub fn uninstall_workflow(
    params: UninstallWorkflowParams,
    home_dir_override: Option<&Path>,
) -> Result<UninstallWorkflowOutcome, String> {
    let trimmed = params.name.trim().to_string();

    let home = match home_dir_override
        .map(|p| p.to_path_buf())
        .or_else(dirs::home_dir)
    {
        Some(h) => h,
        None => return Err("could not resolve user home directory".to_string()),
    };

    // Workflows created post-rename live under `~/.openhuman/workflows/`; older
    // ones under `~/.openhuman/skills/` or the legacy `~/.agents/skills/` root.
    // The first root holding this id wins, matching every user root
    // discover_workflows_inner surfaces (else a listed workflow can't be
    // uninstalled). Slug validation and the symlink/containment/bundle checks
    // are owned by `tinyskills::remove_bundle`.
    let openhuman_dir = home.join(".openhuman");
    let roots: Vec<_> = match crate::skills::write_root::current_agent_skill_home() {
        Some(agent_home) => crate::skills::write_root::agent_user_roots(&agent_home)
            .into_iter()
            .rev()
            .collect(),
        // A SaaS task with no tenant scope must not reach the operator's home.
        None if crate::core::runtime::is_saas() => {
            return Err("no tenant scope for a workflow uninstall".to_string());
        }
        None => vec![
            openhuman_dir.join("workflows"),
            openhuman_dir.join("skills"),
            home.join(".agents").join("skills"),
        ],
    };

    let removed = tinyskills::remove_bundle(&roots, &trimmed).map_err(|e| {
        log::warn!("[skills] uninstall_workflow: refused name={trimmed:?} error={e}");
        match e {
            tinyskills::RemoveError::NotInstalled(slug) => {
                format!("workflow '{slug}' is not installed")
            }
            other => other.to_string(),
        }
    })?;
    log::info!(
        "[skills] uninstall_workflow: removed name={trimmed} path={}",
        removed.display()
    );

    // Notify live agent sessions to drop the removed skill from their
    // `## Installed Skills` catalogue (see `OpenHumanSessionHost::refresh_workflows`).
    crate::skills::ops_discover::invalidate_workflow_metadata_cache();
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::WorkflowsChanged {
        reason: "uninstall".to_string(),
    });

    Ok(UninstallWorkflowOutcome {
        name: trimmed,
        removed_path: removed.display().to_string(),
        scope: WorkflowScope::User,
    })
}
