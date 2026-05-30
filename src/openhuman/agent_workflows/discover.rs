//! Workflow discovery: scanning user + project roots, scope resolution, and
//! collision handling (project shadows user).
//!
//! Mirrors `skills::ops_discover`. User-scope workflows live under
//! `~/.openhuman/workflows/<slug>/`; project-scope under
//! `<workspace>/.openhuman/workflows/<slug>/` and are only loaded when the
//! `<workspace>/.openhuman/trust` marker is present.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::parse::parse_workflow_md;
use super::types::{is_workspace_trusted, Workflow, WorkflowScope, WORKFLOW_MD};

/// Convenience shim: discover workflows for a workspace using the current
/// user's home directory, honoring the trust marker.
pub fn load_workflows(workspace_dir: &Path) -> Vec<Workflow> {
    let trusted = is_workspace_trusted(workspace_dir);
    discover_workflows(dirs::home_dir().as_deref(), Some(workspace_dir), trusted)
}

/// Discover workflows from user and (optionally trusted) project scopes.
///
/// On name collision, project scope wins over user scope. Results are sorted
/// by display name.
pub fn discover_workflows(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
) -> Vec<Workflow> {
    // Scan user first, then project, so project registrations overwrite user
    // ones on name collision.
    let mut by_name: HashMap<String, Workflow> = HashMap::new();

    if let Some(home) = home_dir {
        absorb(
            &mut by_name,
            scan_root(
                &home.join(".openhuman").join("workflows"),
                WorkflowScope::User,
            ),
        );
    }

    if let Some(ws) = workspace_dir {
        if trusted {
            absorb(
                &mut by_name,
                scan_root(
                    &ws.join(".openhuman").join("workflows"),
                    WorkflowScope::Project,
                ),
            );
        } else {
            log::debug!(
                "[workflows] project scope skipped (untrusted workspace) at {}",
                ws.display()
            );
        }
    }

    let mut out: Vec<Workflow> = by_name.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    log::debug!("[workflows] discovered {} workflow(s)", out.len());
    out
}

fn absorb(by_name: &mut HashMap<String, Workflow>, incoming: Vec<Workflow>) {
    for workflow in incoming {
        by_name.insert(workflow.name.clone(), workflow);
    }
}

fn scan_root(root: &Path, scope: WorkflowScope) -> Vec<Workflow> {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    // `read_dir` order is unspecified; sort by directory name for a stable,
    // reproducible scan order.
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    let mut out = Vec::new();
    for entry in entries {
        // Use `file_type()` (not `is_dir()`) so a symlinked child cannot be
        // loaded as a workflow.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        if dir_name.starts_with('.') {
            continue;
        }
        if let Some(workflow) = load_workflow_dir(&entry.path(), &dir_name, scope) {
            out.push(workflow);
        }
    }
    out
}

fn load_workflow_dir(dir: &Path, dir_name: &str, scope: WorkflowScope) -> Option<Workflow> {
    let path: PathBuf = dir.join(WORKFLOW_MD);
    // Use `symlink_metadata` (not `exists()`, which dereferences) so a symlinked
    // WORKFLOW.md inside an otherwise-trusted directory is not followed and read.
    // This keeps the marker-file check consistent with the non-symlink parent-dir
    // guard in `scan_root`.
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => {
            log::debug!(
                "[workflows] skipping non-regular WORKFLOW.md at {}",
                path.display()
            );
            return None;
        }
        Err(_) => return None,
    }
    let (frontmatter, _body, warnings) = match parse_workflow_md(&path) {
        Some(parts) => parts,
        None => {
            log::warn!("[workflows] could not parse {}", path.display());
            return Some(Workflow::from_parts(
                dir_name,
                Default::default(),
                Some(path.clone()),
                scope,
                vec![format!("could not parse {}", path.display())],
            ));
        }
    };
    Some(Workflow::from_parts(
        dir_name,
        frontmatter,
        Some(path),
        scope,
        warnings,
    ))
}

#[cfg(test)]
#[path = "discover_tests.rs"]
mod tests;
