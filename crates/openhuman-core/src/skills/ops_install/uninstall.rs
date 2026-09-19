//! Removing an installed user-scope skill from disk.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::ops_types::{WorkflowScope, MAX_NAME_LEN, SKILL_MD, WORKFLOW_MD};

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
/// Only user-scope uninstalls are supported. Resolution is defensive:
/// canonicalises paths, refuses symlinks, requires SKILL.md to be present.
///
/// `home_dir_override` is for tests; production callers pass `None`.
pub fn uninstall_workflow(
    params: UninstallWorkflowParams,
    home_dir_override: Option<&Path>,
) -> Result<UninstallWorkflowOutcome, String> {
    let trimmed = params.name.trim().to_string();
    if trimmed.is_empty() {
        return Err("skill name is required".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        log::warn!(
            "[skills] uninstall_workflow: rejected name with path separators name={:?}",
            trimmed
        );
        return Err(format!(
            "skill name '{trimmed}' must not contain path separators"
        ));
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(format!(
            "skill name is {} chars (max {MAX_NAME_LEN})",
            trimmed.len()
        ));
    }

    let home = match home_dir_override
        .map(|p| p.to_path_buf())
        .or_else(dirs::home_dir)
    {
        Some(h) => h,
        None => return Err("could not resolve user home directory".to_string()),
    };

    // Workflows created post-rename live under `~/.openhuman/workflows/`; older
    // ones under `~/.openhuman/skills/` or the legacy `~/.agents/skills/` root.
    // Resolve whichever root actually holds this id so delete works regardless
    // of when/where it was authored — and matches every user root
    // discover_workflows_inner surfaces (else a listed workflow can't be
    // uninstalled).
    let openhuman_dir = home.join(".openhuman");
    let root = [
        openhuman_dir.join("workflows"),
        openhuman_dir.join("skills"),
        home.join(".agents").join("skills"),
    ]
    .into_iter()
    .find(|r| r.join(&trimmed).exists());
    let root = match root {
        Some(r) => r,
        None => return Err(format!("workflow '{trimmed}' is not installed")),
    };

    let root_meta = std::fs::symlink_metadata(&root)
        .map_err(|e| format!("stat {} failed: {e}", root.display()))?;
    if root_meta.file_type().is_symlink() {
        log::warn!(
            "[workflows] uninstall_workflow: refused symlinked root path={}",
            root.display()
        );
        return Err(format!(
            "workflows root {} is a symlink — refusing to resolve",
            root.display()
        ));
    }

    let canonical_root = std::fs::canonicalize(&root)
        .map_err(|e| format!("canonicalize {} failed: {e}", root.display()))?;

    let candidate = root.join(&trimmed);
    match std::fs::symlink_metadata(&candidate) {
        Ok(m) if m.file_type().is_symlink() => {
            log::warn!(
                "[skills] uninstall_workflow: refused symlinked alias name={trimmed} path={}",
                candidate.display()
            );
            return Err(format!(
                "skill '{trimmed}' is a symlinked alias — refusing to resolve"
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("skill '{trimmed}' is not installed"));
        }
        Err(e) => {
            return Err(format!("stat {} failed: {e}", candidate.display()));
        }
    }

    let canonical_candidate = std::fs::canonicalize(&candidate).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("skill '{trimmed}' is not installed")
        } else {
            format!("canonicalize {} failed: {e}", candidate.display())
        }
    })?;

    if !canonical_candidate.starts_with(&canonical_root) {
        log::warn!(
            "[skills] uninstall_workflow: path escape rejected candidate={} root={}",
            canonical_candidate.display(),
            canonical_root.display()
        );
        return Err(format!(
            "refused to remove {} — path escapes skills root",
            canonical_candidate.display()
        ));
    }

    let meta = std::fs::symlink_metadata(&canonical_candidate)
        .map_err(|e| format!("stat {} failed: {e}", canonical_candidate.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a directory — refusing to remove",
            canonical_candidate.display()
        ));
    }

    if !canonical_candidate.join(WORKFLOW_MD).exists()
        && !canonical_candidate.join(SKILL_MD).exists()
    {
        return Err(format!(
            "{} does not look like a workflow (missing {WORKFLOW_MD})",
            canonical_candidate.display()
        ));
    }

    log::info!(
        "[skills] uninstall_workflow: removing name={trimmed} path={}",
        canonical_candidate.display()
    );
    std::fs::remove_dir_all(&canonical_candidate)
        .map_err(|e| format!("remove {} failed: {e}", canonical_candidate.display()))?;

    // Notify live agent sessions to drop the removed skill from their
    // `## Installed Skills` catalogue (see `OpenHumanSessionHost::refresh_workflows`).
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::WorkflowsChanged {
        reason: "uninstall".to_string(),
    });

    Ok(UninstallWorkflowOutcome {
        name: trimmed,
        removed_path: canonical_candidate.display().to_string(),
        scope: WorkflowScope::User,
    })
}
