//! Public discovery entry points: the workspace-trust check and the handful
//! of `load_workflow_metadata*` / `discover_workflows*` shims that select a
//! root scan (see [`super::scan`]) for a given caller shape.

use std::path::{Path, PathBuf};

use crate::skills::ops_types::{Workflow, TRUST_MARKER};

use super::scan::{discover_filtered, ALL_ROOT_KINDS, WORKFLOW_ROOT_KINDS};

/// Initialize the legacy skills directory in the specified workspace.
///
/// Creates `<workspace>/skills/` and a placeholder `README.md` so the folder
/// is visible to the user. New-style skills should live under
/// `<workspace>/.openhuman/skills/` instead, but this directory is kept for
/// backward compatibility.
pub fn init_workflows_dir(workspace_dir: &Path) -> Result<(), String> {
    let skills_dir = workspace_dir.join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| {
        format!(
            "failed to create skills directory {}: {e}",
            skills_dir.display()
        )
    })?;

    let readme_path = skills_dir.join("README.md");
    if !readme_path.exists() {
        let content = "# Skills\n\nPut one skill per directory under this folder.\n";
        std::fs::write(&readme_path, content)
            .map_err(|e| format!("failed to write {}: {e}", readme_path.display()))?;
    }

    Ok(())
}

/// The home directory skill discovery scans for user-scope roots
/// (`~/.openhuman/skills/`, `~/.agents/skills/`, `~/.openhuman/workflows/`).
///
/// `dirs::home_dir()` for every ordinary caller. `None` when the ambient
/// [`CoreContext`](crate::core::runtime::CoreContext) was derived with
/// `user_skill_roots = false` — an embedded agent whose host installed its
/// skills explicitly and must not see the operator's own — so the caller's
/// discovery pipeline scans no user scope at all, exactly as passing `None`
/// for `home_dir` always has.
pub fn discovery_home_dir() -> Option<PathBuf> {
    if crate::core::runtime::CoreContext::current_user_skill_roots() {
        dirs::home_dir()
    } else {
        log::debug!("[skills][discover] user-scope roots hidden by the ambient context");
        None
    }
}

/// Backwards-compatible shim for callers that only have a workspace path.
///
/// Delegates to [`discover_workflows`] with the current user's home directory
/// so user-scope skills (`~/.openhuman/skills/`, `~/.agents/skills/`) are
/// surfaced for existing production callers (`agent::session_host::builder`,
/// `channels::runtime::startup`). Previously this shim passed `None` for the
/// home directory, which silently dropped user-installed skills from the
/// main runtime path.
///
/// Project-scope (workspace) skills still take precedence over user-scope
/// on name collisions.
pub fn load_workflow_metadata(workspace_dir: &Path) -> Vec<Workflow> {
    let trusted = is_workspace_trusted(workspace_dir);
    let home = discovery_home_dir();
    discover_workflows_inner(home.as_deref(), Some(workspace_dir), trusted)
}

/// Discover skills from every supported location.
///
/// * `home_dir` — user home (typically `dirs::home_dir()`), scanned for
///   `~/.openhuman/skills/` and `~/.agents/skills/`.
/// * `workspace_dir` — current workspace, scanned for project-scope paths.
/// * `trusted` — whether the caller has verified the project trust marker.
///   Project-scope skills are silently skipped when `false`.
///
/// On name collisions, project-scope wins over user-scope and a warning is
/// attached to the retained skill.
pub fn discover_workflows(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
) -> Vec<Workflow> {
    #[cfg(test)]
    DISCOVERY_CALLS.with(|c| c.set(c.get() + 1));
    discover_workflows_inner(home_dir, workspace_dir, trusted)
}

#[cfg(test)]
thread_local! {
    /// Test-only counter of full on-disk discovery passes made on this thread.
    /// Discovery re-reads and re-parses every skill bundle under every root, so
    /// a caller that runs it twice for one lookup pays the whole tree twice
    /// (#6166). Thread-local so parallel tests can't perturb each other's count.
    pub(crate) static DISCOVERY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether the workspace has opted into loading project-scope skills.
///
/// Looks for `<workspace>/.openhuman/trust`. The marker file's contents are
/// ignored — presence is sufficient.
pub fn is_workspace_trusted(workspace_dir: &Path) -> bool {
    workspace_dir.join(".openhuman").join(TRUST_MARKER).exists()
}

pub(crate) fn discover_workflows_inner(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
) -> Vec<Workflow> {
    discover_filtered(home_dir, workspace_dir, trusted, ALL_ROOT_KINDS)
}

/// Discover only automation bundles under the `workflows/` roots.
/// Capability skills are deliberately excluded; they remain available to the
/// agent harness and run/describe paths.
///
/// Note: bundles authored *before* the skills→workflows rename live under the
/// `skills/` roots and will therefore not appear in this automations-only view;
/// new automations created via "New workflow" land in `~/.openhuman/workflows/`.
pub fn discover_automations(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
) -> Vec<Workflow> {
    tracing::debug!(
        trusted,
        has_home = home_dir.is_some(),
        has_workspace = workspace_dir.is_some(),
        "[workflows] discover:automations:enter"
    );
    discover_filtered(home_dir, workspace_dir, trusted, WORKFLOW_ROOT_KINDS)
}
