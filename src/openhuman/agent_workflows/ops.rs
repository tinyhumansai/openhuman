//! Workflow lifecycle operations: create, read, uninstall.
//!
//! Public entry points operate against the user-scope root
//! (`~/.openhuman/workflows/`). The `*_in_root` inner variants take an explicit
//! root so tests can exercise the full create/read/uninstall cycle against a
//! temp directory without touching the real home (mirrors `skills::create_skill_inner`).

use std::path::{Path, PathBuf};

use super::parse::parse_workflow_md;
use super::types::WORKFLOW_MD;
use super::{Workflow, WorkflowScope};

/// Resolve the user-scope workflows root: `~/.openhuman/workflows/`.
fn user_workflows_root() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("could not resolve home directory")?;
    Ok(home.join(".openhuman").join("workflows"))
}

/// Create a new user-scope workflow by scaffolding a `WORKFLOW.md`.
pub fn create_workflow(
    name: &str,
    description: &str,
    when_to_use: &str,
) -> Result<Workflow, String> {
    let root = user_workflows_root()?;
    create_workflow_in_root(&root, name, description, when_to_use)
}

pub(crate) fn create_workflow_in_root(
    root: &Path,
    name: &str,
    description: &str,
    when_to_use: &str,
) -> Result<Workflow, String> {
    let slug = slugify(name);
    if slug.is_empty() {
        return Err("workflow name must contain alphanumeric characters".to_string());
    }
    let dir = root.join(&slug);
    let path = dir.join(WORKFLOW_MD);
    if path.exists() {
        return Err(format!("workflow '{slug}' already exists"));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create workflow dir {}: {e}", dir.display()))?;
    let body = scaffold_body(name, description, when_to_use);
    std::fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    log::info!(
        "[workflows] created workflow slug={slug} at {}",
        path.display()
    );
    let (frontmatter, _body, warnings) = parse_workflow_md(&path)
        .ok_or_else(|| "failed to re-parse scaffolded workflow".to_string())?;
    Ok(Workflow::from_parts(
        slug,
        frontmatter,
        Some(path),
        WorkflowScope::User,
        warnings,
    ))
}

fn scaffold_body(name: &str, description: &str, when_to_use: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nwhen_to_use: {when_to_use}\nphases:\n  on_pick_up_task:\n    rules:\n      - Describe what to do when this task starts.\n---\n# {name}\n\n{description}\n"
    )
}

/// Read a workflow by id (dir name) returning the full parsed workflow.
pub fn read_workflow(id: &str) -> Result<Workflow, String> {
    let root = user_workflows_root()?;
    read_workflow_in_root(&root, id)
}

pub(crate) fn read_workflow_in_root(root: &Path, id: &str) -> Result<Workflow, String> {
    if id.trim().is_empty() {
        return Err("workflow id must not be empty".to_string());
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("invalid workflow id".to_string());
    }
    let path = root.join(id).join(WORKFLOW_MD);
    if !path.exists() {
        return Err(format!("workflow '{id}' not found"));
    }
    let (frontmatter, _body, warnings) =
        parse_workflow_md(&path).ok_or_else(|| format!("failed to parse workflow '{id}'"))?;
    Ok(Workflow::from_parts(
        id.to_string(),
        frontmatter,
        Some(path),
        WorkflowScope::User,
        warnings,
    ))
}

/// Uninstall a user-scope workflow by id, hardened against path traversal.
pub fn uninstall_workflow(id: &str) -> Result<bool, String> {
    let root = user_workflows_root()?;
    uninstall_workflow_in_root(&root, id)
}

pub(crate) fn uninstall_workflow_in_root(root: &Path, id: &str) -> Result<bool, String> {
    if id.trim().is_empty() {
        return Err("workflow id must not be empty".to_string());
    }
    // Reject ids with separators or traversal before touching the filesystem.
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("invalid workflow id".to_string());
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| format!("failed to canonicalize workflows root: {e}"))?;
    let dir = canonical_root.join(id);
    let canonical_dir =
        std::fs::canonicalize(&dir).map_err(|e| format!("workflow '{id}' not found: {e}"))?;
    if !canonical_dir.starts_with(&canonical_root) {
        return Err("workflow path escapes root".to_string());
    }
    std::fs::remove_dir_all(&canonical_dir)
        .map_err(|e| format!("failed to remove workflow '{id}': {e}"))?;
    log::info!("[workflows] uninstalled workflow id={id}");
    Ok(true)
}

/// Slugify a workflow name into a filesystem-safe directory id.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
use slugify as slugify_for_test;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
