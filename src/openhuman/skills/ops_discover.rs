//! Workflow discovery: scanning root directories, scope resolution, collision handling,
//! and skill resource reading.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::ops_parse::{load_from_legacy_manifest, load_from_workflow_md};
use super::ops_types::{
    Workflow, WorkflowScope, MAX_WORKFLOW_RESOURCE_BYTES, SKILL_JSON, SKILL_MD, TRUST_MARKER,
    WORKFLOW_MD,
};

const EXCLUDED_SKILL_DIRS: &[&str] = &[
    ".git",
    ".github",
    ".hub",
    ".archive",
    ".venv",
    "venv",
    "node_modules",
    "site-packages",
    "__pycache__",
    ".tox",
    ".nox",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
];

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

/// Backwards-compatible shim for callers that only have a workspace path.
///
/// Delegates to [`discover_workflows`] with the current user's home directory
/// so user-scope skills (`~/.openhuman/skills/`, `~/.agents/skills/`) are
/// surfaced for existing production callers (`agent::harness::session::builder`,
/// `channels::runtime::startup`). Previously this shim passed `None` for the
/// home directory, which silently dropped user-installed skills from the
/// main runtime path.
///
/// Project-scope (workspace) skills still take precedence over user-scope
/// on name collisions.
pub fn load_workflow_metadata(workspace_dir: &Path) -> Vec<Workflow> {
    let trusted = is_workspace_trusted(workspace_dir);
    let home = dirs::home_dir();
    discover_workflows_inner(home.as_deref(), Some(workspace_dir), None, trusted)
}

/// Like [`load_workflow_metadata`], but additionally scans a profile-local
/// skills root (`<workspace>/personalities/<id>/skills/`) when one is supplied.
///
/// Callers pass the active profile's root (resolved via
/// `profiles::profile_skills_root`) so the returned catalog carries that
/// profile's private skills. `None` reproduces [`load_workflow_metadata`]
/// byte-for-byte, so the profile-less session and every other profile are
/// unaffected. Profile-local skills win same-name collisions against global
/// scopes (see [`WorkflowScope::Profile`]).
pub fn load_workflow_metadata_for_profile(
    workspace_dir: &Path,
    profile_skills_root: Option<&Path>,
) -> Vec<Workflow> {
    let trusted = is_workspace_trusted(workspace_dir);
    let home = dirs::home_dir();
    discover_workflows_inner(
        home.as_deref(),
        Some(workspace_dir),
        profile_skills_root,
        trusted,
    )
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
    discover_workflows_inner(home_dir, workspace_dir, None, trusted)
}

/// Discover skills including a profile-local root, for a turn running under a
/// specific agent profile.
///
/// `profile_skills_root` is `<workspace>/personalities/<id>/skills/` (resolved
/// via `profiles::profile_skills_root`, which validates the id). It is scanned
/// unconditionally — no trust marker is required, since the directory is
/// core-managed under `workspace_dir` — and its bundles win same-name collisions
/// against every global scope for this profile. `None` is identical to
/// [`discover_workflows`], so other profiles and the default session never see
/// these skills.
pub fn discover_workflows_with_profile(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    profile_skills_root: Option<&Path>,
    trusted: bool,
) -> Vec<Workflow> {
    discover_workflows_inner(home_dir, workspace_dir, profile_skills_root, trusted)
}

/// Whether the workspace has opted into loading project-scope skills.
///
/// Looks for `<workspace>/.openhuman/trust`. The marker file's contents are
/// ignored — presence is sufficient.
pub fn is_workspace_trusted(workspace_dir: &Path) -> bool {
    workspace_dir.join(".openhuman").join(TRUST_MARKER).exists()
}

/// Which on-disk root category a bundle was discovered under.
///
/// `Workflow` roots (`.openhuman/workflows/`) hold task *automations* authored
/// via "New workflow". `Skill` roots (`.openhuman/skills/`, `.agents/skills/`,
/// and the legacy `<workspace>/skills/`) hold capability *skills*. Both are the
/// same on-disk primitive (SKILL.md / WORKFLOW.md bundles) and the agent
/// harness loads both — but the Automations UI lists only `Workflow`-root
/// bundles (see [`discover_automations`]) so capability skills don't masquerade
/// as task templates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RootKind {
    Skill,
    Workflow,
}

const ALL_ROOT_KINDS: &[RootKind] = &[RootKind::Skill, RootKind::Workflow];
const WORKFLOW_ROOT_KINDS: &[RootKind] = &[RootKind::Workflow];

pub(crate) fn discover_workflows_inner(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    profile_skills_root: Option<&Path>,
    trusted: bool,
) -> Vec<Workflow> {
    discover_filtered(
        home_dir,
        workspace_dir,
        profile_skills_root,
        trusted,
        ALL_ROOT_KINDS,
    )
}

/// Discover only *automation* bundles — those under the `workflows/` roots —
/// for the Automations UI list (`openhuman.skills_list`).
///
/// Capability skills (under the `skills/` / `.agents/skills/` / legacy
/// `<workspace>/skills/` roots) are deliberately excluded so they don't show up
/// as task templates. They remain fully available to the agent harness and the
/// run/describe paths via [`discover_workflows`] / [`load_workflow_metadata`].
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
    discover_filtered(home_dir, workspace_dir, None, trusted, WORKFLOW_ROOT_KINDS)
}

/// Shared discovery core. `kinds` selects which root categories to scan,
/// letting the full surface ([`discover_workflows_inner`]) and the
/// automations-only list ([`discover_automations`]) share collision handling.
fn discover_filtered(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    profile_skills_root: Option<&Path>,
    trusted: bool,
    kinds: &[RootKind],
) -> Vec<Workflow> {
    tracing::debug!(
        trusted,
        has_home = home_dir.is_some(),
        has_workspace = workspace_dir.is_some(),
        has_profile_root = profile_skills_root.is_some(),
        include_skills = kinds.contains(&RootKind::Skill),
        include_workflows = kinds.contains(&RootKind::Workflow),
        "[workflows] discover:enter"
    );
    // Scan order matters for collision resolution: the last scope to register
    // a name wins, so we scan user first, then project, then legacy.
    let mut by_name: HashMap<String, Workflow> = HashMap::new();

    if let Some(home) = home_dir {
        for (root, kind) in user_roots(home) {
            if kinds.contains(&kind) {
                tracing::trace!(
                    root = %root.display(),
                    ?kind,
                    scope = ?WorkflowScope::User,
                    "[workflows] discover:branch:user"
                );
                absorb(&mut by_name, scan_root(&root, WorkflowScope::User));
            }
        }
    }

    if let Some(ws) = workspace_dir {
        if trusted {
            for (root, kind) in project_roots(ws) {
                if kinds.contains(&kind) {
                    tracing::trace!(
                        root = %root.display(),
                        ?kind,
                        scope = ?WorkflowScope::Project,
                        "[workflows] discover:branch:project"
                    );
                    absorb(&mut by_name, scan_root(&root, WorkflowScope::Project));
                }
            }
        }
        // Legacy `<workspace>/skills/` is a skill root: scanned for the full
        // surface (back-compat, no trust marker required) but excluded from the
        // automations-only view. Flagged with `legacy = true` so the UI can
        // nudge migration.
        if kinds.contains(&RootKind::Skill) {
            let legacy_root = ws.join("skills");
            tracing::trace!(
                root = %legacy_root.display(),
                scope = ?WorkflowScope::Legacy,
                "[workflows] discover:branch:legacy"
            );
            absorb(&mut by_name, scan_root(&legacy_root, WorkflowScope::Legacy));
        }
    }

    // Profile-local skills (`<workspace>/personalities/<id>/skills/`) are a skill
    // root scoped to the *active* profile: scanned last and at the highest
    // precedence so a profile-local bundle wins any same-name collision against
    // the global scopes for its owner (see [`precedence`]). Excluded from the
    // automations-only view for the same reason as the legacy skill root. No
    // trust marker is consulted — the directory is core-managed under
    // `workspace_dir`, seeded by `ensure_profile_home`.
    if let Some(profile_root) = profile_skills_root {
        if kinds.contains(&RootKind::Skill) {
            tracing::debug!(
                root = %profile_root.display(),
                scope = ?WorkflowScope::Profile,
                "[profiles] discover:branch:profile-local skills"
            );
            let before = by_name.len();
            absorb(
                &mut by_name,
                scan_root(profile_root, WorkflowScope::Profile),
            );
            tracing::debug!(
                names_before = before,
                names_after = by_name.len(),
                "[profiles] profile-local skills absorbed (profile scope wins same-name collisions)"
            );
        }
    }

    let mut out: Vec<Workflow> = by_name.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    tracing::debug!(discovered_count = out.len(), "[workflows] discover:exit");
    out
}

fn user_roots(home: &Path) -> Vec<(PathBuf, RootKind)> {
    // `workflows/` is the current layout (create writes here); the `skills/`
    // roots are still scanned for back-compat with installs created before the
    // skills→workflows rename. Order matters: `workflows/` is scanned last so a
    // same-named entry there wins over a legacy `skills/` one.
    vec![
        (home.join(".openhuman").join("skills"), RootKind::Skill),
        (home.join(".agents").join("skills"), RootKind::Skill),
        (
            home.join(".openhuman").join("workflows"),
            RootKind::Workflow,
        ),
    ]
}

fn project_roots(workspace: &Path) -> Vec<(PathBuf, RootKind)> {
    vec![
        (workspace.join(".openhuman").join("skills"), RootKind::Skill),
        (workspace.join(".agents").join("skills"), RootKind::Skill),
        (
            workspace.join(".openhuman").join("workflows"),
            RootKind::Workflow,
        ),
    ]
}

fn absorb(by_name: &mut HashMap<String, Workflow>, incoming: Vec<Workflow>) {
    for mut skill in incoming {
        let key = skill.name.clone();
        // A workflow's runnable identity is `dir_name`, while `name` is only
        // display metadata. Collapse on either so a profile-local `foo/` also
        // shadows a global `foo/` whose frontmatter happens to use a different
        // display name. Otherwise registry lookup by slug could nondeterministically
        // select the global copy.
        let collision_keys: Vec<String> = by_name
            .iter()
            .filter(|(existing_name, existing)| {
                existing_name.as_str() == key || existing.dir_name == skill.dir_name
            })
            .map(|(existing_name, _)| existing_name.clone())
            .collect();

        if let Some((_, highest_name, highest_scope)) = collision_keys
            .iter()
            .filter_map(|collision_key| by_name.get(collision_key))
            .map(|existing| {
                (
                    precedence(existing.scope),
                    existing.name.clone(),
                    existing.scope,
                )
            })
            .max_by_key(|(rank, _, _)| *rank)
        {
            if precedence(skill.scope) < precedence(highest_scope) {
                if let Some(kept) = by_name.get_mut(&highest_name) {
                    kept.warnings.push(format!(
                        "workflow id '{}' or name '{}' also declared in {:?} scope at {} (ignored)",
                        skill.dir_name,
                        skill.name,
                        skill.scope,
                        skill
                            .location
                            .as_deref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<unknown>".to_string())
                    ));
                }
                continue;
            }
        }

        for collision_key in collision_keys {
            if let Some(loser) = by_name.remove(&collision_key) {
                skill.warnings.push(format!(
                    "shadowed {:?}-scope skill '{}' (workflow id '{}') at {}",
                    loser.scope,
                    loser.name,
                    loser.dir_name,
                    loser
                        .location
                        .as_deref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<unknown>".to_string())
                ));
            }
        }
        by_name.insert(key, skill);
    }
}

fn precedence(scope: WorkflowScope) -> u8 {
    match scope {
        WorkflowScope::Legacy => 0,
        WorkflowScope::User => 1,
        WorkflowScope::Project => 2,
        // Profile-local skills win against every global scope for their owner.
        WorkflowScope::Profile => 3,
    }
}

fn scan_root(root: &Path, scope: WorkflowScope) -> Vec<Workflow> {
    let mut out = Vec::new();
    scan_root_inner(root, scope, &mut out);
    out.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    out
}

fn scan_root_inner(root: &Path, scope: WorkflowScope, out: &mut Vec<Workflow>) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    // `read_dir` order is unspecified. When two sibling directories declare
    // the same logical `frontmatter.name` (which can differ from the folder
    // name), cross-scope/same-scope deduplication downstream would otherwise
    // pick a non-deterministic winner across runs. Sort by on-disk directory
    // name for a stable, reproducible order.
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        // Use `file_type()` rather than `path.is_dir()` so a symlinked
        // child cannot be loaded as a skill. `is_dir()` dereferences
        // symlinks, which would re-open out-of-tree loading even though
        // `walk_files` already rejects symlinks deeper in the resource
        // walker. Skip both symlinks and non-directory entries here; if
        // the `file_type()` call itself fails (rare — transient I/O),
        // treat it as "not safe to traverse" and skip.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        let dir_name = entry.file_name().to_string_lossy().to_string();
        if dir_name.starts_with('.') || EXCLUDED_SKILL_DIRS.contains(&dir_name.as_str()) {
            continue;
        }
        if let Some(skill) = load_skill_dir(&path, &dir_name, scope) {
            out.push(skill);
            continue;
        }
        scan_root_inner(&path, scope, out);
    }
}

fn load_skill_dir(dir: &Path, dir_name: &str, scope: WorkflowScope) -> Option<Workflow> {
    // WORKFLOW.md is the current filename; SKILL.md is read for back-compat
    // with workflows authored before the rename.
    let workflow_md = dir.join(WORKFLOW_MD);
    let legacy_md = dir.join(SKILL_MD);
    let legacy_manifest = dir.join(SKILL_JSON);

    // `exists()` follows symlinks, so a manifest could point at an arbitrary
    // file outside the bundle and discovery would ingest its contents into the
    // catalog/prompt flow. Since the legacy `skills/` roots are scanned without
    // a trust marker, require a real (non-symlink) regular file before loading.
    let is_safe_manifest = |path: &Path| {
        matches!(
            std::fs::symlink_metadata(path),
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink()
        )
    };

    if is_safe_manifest(&workflow_md) {
        return Some(load_from_workflow_md(&workflow_md, dir, dir_name, scope));
    }
    if is_safe_manifest(&legacy_md) {
        return Some(load_from_workflow_md(&legacy_md, dir, dir_name, scope));
    }
    if is_safe_manifest(&legacy_manifest) {
        return Some(load_from_legacy_manifest(
            &legacy_manifest,
            dir,
            dir_name,
            scope,
        ));
    }
    None
}

/// Read a bundled skill resource as UTF-8 text, hardened against directory
/// traversal, symlink escape, and oversized payloads.
///
/// `skill_id` identifies the skill by its discovered `name` or its on-disk
/// `dir_name` slug — the same identifiers surfaced in the UI summary. The
/// skill is resolved by running the standard
/// discovery pipeline (`dirs::home_dir()` + `workspace_dir`, honoring the
/// `.openhuman/trust` marker) and locating the matching entry; this keeps the
/// read scoped to legitimately installed skills and reuses all the symlink /
/// traversal hardening already baked into discovery.
///
/// `relative_path` is resolved relative to the skill's on-disk directory
/// (the parent of its `SKILL.md` / `skill.json`). All of the following are
/// rejected with an error:
///
/// * paths that canonicalize outside the skill root (traversal),
/// * paths whose final component or any intermediate component is a symlink
///   (link-follow escape),
/// * non-file targets (directories, sockets, fifos),
/// * files larger than [`MAX_WORKFLOW_RESOURCE_BYTES`],
/// * non-UTF-8 byte contents (binary files must be surfaced some other way —
///   no lossy replacement).
///
/// On success returns the file's contents as an owned `String`.
pub fn read_workflow_resource(
    workspace_dir: &Path,
    skill_id: &str,
    relative_path: &Path,
) -> Result<String, String> {
    read_workflow_resource_with_profile(workspace_dir, skill_id, relative_path, None)
}

/// The dir_name/name set of skills discovered under a profile-local skills root.
///
/// Used by the `describe_workflow` / `read_workflow_resource` / `run_workflow`
/// tools to treat a profile's private skills as implicitly allowed for their
/// owner (they bypass the `allowed_skills` allowlist, mirroring `list_workflows`).
/// Empty when no profile root is active, so the profile-less session and other
/// profiles are unaffected.
pub fn profile_local_skill_ids(
    profile_skills_root: Option<&Path>,
) -> std::collections::HashSet<String> {
    let Some(root) = profile_skills_root else {
        return std::collections::HashSet::new();
    };
    scan_root(root, WorkflowScope::Profile)
        .into_iter()
        .flat_map(|w| {
            let mut ids = vec![w.name];
            if !w.dir_name.is_empty() {
                ids.push(w.dir_name);
            }
            ids
        })
        .collect()
}

/// Like [`read_workflow_resource`], but resolves the skill against the active
/// profile's private skills root too (`<workspace>/personalities/<id>/skills/`)
/// when `profile_skills_root` is supplied. `None` is byte-identical to
/// [`read_workflow_resource`].
pub fn read_workflow_resource_with_profile(
    workspace_dir: &Path,
    skill_id: &str,
    relative_path: &Path,
    profile_skills_root: Option<&Path>,
) -> Result<String, String> {
    tracing::debug!(
        skill_id = %skill_id,
        relative_path = %relative_path.display(),
        workspace = %workspace_dir.display(),
        has_profile_root = profile_skills_root.is_some(),
        "[skills] read_workflow_resource: entry"
    );

    if skill_id.trim().is_empty() {
        return Err("skill_id must not be empty".to_string());
    }

    let relative_str = relative_path.to_string_lossy();
    if relative_str.trim().is_empty() {
        return Err("relative_path must not be empty".to_string());
    }
    if relative_path.is_absolute() {
        return Err("relative_path must be relative, not absolute".to_string());
    }
    // Reject any component that is `..`, is empty, starts with `.`, or is the
    // root. `..` is the obvious traversal vector; the others are defense in
    // depth against unusual path inputs (e.g. `./`, `//foo`, Windows `C:`).
    for component in relative_path.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) => {}
            Component::ParentDir => {
                return Err("relative_path must not contain '..' components".to_string());
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {
                return Err("relative_path must be a plain relative path".to_string());
            }
        }
    }

    // Resolve the skill by running the standard discovery pipeline. We reuse
    // `load_workflow_metadata_for_profile` (which honors both user and workspace
    // roots plus the trust marker, and the active profile's private root when
    // supplied) so the resource read is scoped to the exact same set of skills
    // the owner would already have seen listed.
    let skill = resolve_workflow_for_resource(
        load_workflow_metadata_for_profile(workspace_dir, profile_skills_root),
        skill_id,
    )?;
    let skill_root = skill
        .location
        .as_deref()
        .and_then(|p| p.parent())
        .ok_or_else(|| format!("skill '{skill_id}' has no on-disk location"))?
        .to_path_buf();

    // Canonicalize the root first. The root must itself be a real directory
    // on disk (not a symlink). Reject early if this fails.
    let canonical_root = std::fs::canonicalize(&skill_root).map_err(|e| {
        format!(
            "failed to canonicalize skill root {}: {e}",
            skill_root.display()
        )
    })?;

    let requested = canonical_root.join(relative_path);

    // Pre-check the immediate target with `symlink_metadata` so we catch
    // symlinked leaves before `canonicalize` silently follows them.
    let leaf_meta = std::fs::symlink_metadata(&requested)
        .map_err(|e| format!("failed to stat resource {}: {e}", requested.display()))?;
    if leaf_meta.file_type().is_symlink() {
        return Err("resource path is a symlink".to_string());
    }
    if !leaf_meta.is_file() {
        return Err("resource path is not a regular file".to_string());
    }

    // Size gate — check via metadata before reading so we never allocate the
    // buffer for an oversized file.
    let size = leaf_meta.len();
    if size > MAX_WORKFLOW_RESOURCE_BYTES {
        return Err(format!(
            "resource file is {size} bytes, exceeds limit of {MAX_WORKFLOW_RESOURCE_BYTES}"
        ));
    }

    // Canonicalize the full path and verify it stays within the skill root.
    // This catches any symlink reachable via an intermediate path component
    // that was created after our initial checks (race-ish, but the
    // `is_symlink` check above makes the obvious attack infeasible).
    let canonical_requested = std::fs::canonicalize(&requested).map_err(|e| {
        format!(
            "failed to canonicalize resource {}: {e}",
            requested.display()
        )
    })?;
    if !canonical_requested.starts_with(&canonical_root) {
        return Err(format!(
            "resource path escapes skill root: {}",
            canonical_requested.display()
        ));
    }

    // Read the bytes and enforce strict UTF-8 (no lossy replacement — we
    // would rather refuse a binary file than silently mangle it).
    let bytes = std::fs::read(&canonical_requested).map_err(|e| {
        format!(
            "failed to read resource {}: {e}",
            canonical_requested.display()
        )
    })?;
    let content = std::str::from_utf8(&bytes)
        .map_err(|e| format!("resource is not valid UTF-8 text: {e}"))?
        .to_string();

    tracing::debug!(
        skill_id = %skill_id,
        bytes = bytes.len(),
        "[skills] read_workflow_resource: success"
    );

    Ok(content)
}

fn resolve_workflow_for_resource(
    workflows: Vec<Workflow>,
    skill_id: &str,
) -> Result<Workflow, String> {
    let mut dir_match: Option<Workflow> = None;
    let mut name_match: Option<Workflow> = None;

    for workflow in workflows {
        if workflow.dir_name == skill_id {
            if dir_match.is_some() {
                return Err(format!(
                    "skill id '{skill_id}' is ambiguous across multiple skill directories"
                ));
            }
            dir_match = Some(workflow);
            continue;
        }

        if workflow.name == skill_id {
            if name_match.is_some() {
                return Err(format!(
                    "skill name '{skill_id}' is ambiguous; use the directory id"
                ));
            }
            name_match = Some(workflow);
        }
    }

    match (dir_match, name_match) {
        (Some(dir_skill), Some(name_skill)) => {
            if dir_skill.location == name_skill.location {
                Ok(dir_skill)
            } else {
                Err(format!(
                    "skill id '{skill_id}' matches both a directory id and a different skill name"
                ))
            }
        }
        (Some(skill), None) | (None, Some(skill)) => Ok(skill),
        (None, None) => Err(format!("skill '{skill_id}' not found")),
    }
}

#[cfg(test)]
mod include_skills_tests {
    use super::*;

    /// Write a minimal `<file>`-named bundle under `root/slug/`.
    fn seed_bundle(root: &Path, slug: &str, file: &str) {
        let dir = root.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(file),
            format!("---\nname: {slug}\ndescription: {slug} desc\n---\n\n{slug} body\n"),
        )
        .unwrap();
    }

    /// `discover_automations` lists only `workflows/`-root automations, while
    /// `discover_workflows` additionally surfaces `skills/`-root installs. This
    /// is exactly the branch `handle_skills_list` selects on `include_skills`
    /// so the Skills Explorer's Installed tab can show registry installs (#3954).
    #[test]
    fn automations_excludes_skill_roots_but_full_discover_includes_them() {
        let home = tempfile::TempDir::new().unwrap();
        let home_path = home.path();
        // A registry-style install lands under `~/.openhuman/skills/`.
        seed_bundle(
            &home_path.join(".openhuman").join("skills"),
            "installed-skill",
            "SKILL.md",
        );
        // A "New workflow" automation lands under `~/.openhuman/workflows/`.
        seed_bundle(
            &home_path.join(".openhuman").join("workflows"),
            "my-automation",
            "WORKFLOW.md",
        );

        // Automations-only view (the default `skills_list` path) hides the skill.
        let automations = discover_automations(Some(home_path), None, false);
        let auto_names: Vec<&str> = automations.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(
            auto_names,
            vec!["my-automation"],
            "discover_automations must exclude `skills/`-root installs"
        );

        // Full view (`include_skills=true`) surfaces both.
        let full = discover_workflows(Some(home_path), None, false);
        let mut full_names: Vec<&str> = full.iter().map(|w| w.name.as_str()).collect();
        full_names.sort_unstable();
        assert_eq!(
            full_names,
            vec!["installed-skill", "my-automation"],
            "discover_workflows must include `skills/`-root installs"
        );
    }
}

#[cfg(test)]
mod profile_scope_tests {
    use super::*;

    /// Write a minimal `WORKFLOW.md` bundle under `root/slug/`.
    fn seed_bundle(root: &Path, slug: &str) {
        seed_bundle_with_name(root, slug, slug);
    }

    fn seed_bundle_with_name(root: &Path, slug: &str, name: &str) {
        let dir = root.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("WORKFLOW.md"),
            format!("---\nname: {name}\ndescription: {name} desc\n---\n\n{name} body\n"),
        )
        .unwrap();
    }

    /// Profile-local skills appear ONLY when their root is passed, and never for
    /// the profile-less session or a *different* profile's root (2a scoping
    /// matrix).
    #[test]
    fn profile_local_skills_scoped_to_their_owner() {
        let home = tempfile::TempDir::new().unwrap();
        // A global user-scope skill everyone sees.
        seed_bundle(
            &home.path().join(".openhuman").join("skills"),
            "global-skill",
        );

        // Two distinct profile roots (alice / bob), each with a private skill.
        let alice_root = tempfile::TempDir::new().unwrap();
        seed_bundle(alice_root.path(), "alice-only");
        let bob_root = tempfile::TempDir::new().unwrap();
        seed_bundle(bob_root.path(), "bob-only");

        let names = |workflows: Vec<Workflow>| {
            let mut n: Vec<String> = workflows.into_iter().map(|w| w.name).collect();
            n.sort();
            n
        };

        // No profile: only the global skill.
        let none = names(discover_workflows_with_profile(
            Some(home.path()),
            None,
            None,
            false,
        ));
        assert_eq!(none, vec!["global-skill"]);

        // Alice's turn: global + alice-only, never bob-only.
        let alice = names(discover_workflows_with_profile(
            Some(home.path()),
            None,
            Some(alice_root.path()),
            false,
        ));
        assert_eq!(alice, vec!["alice-only", "global-skill"]);

        // Bob's turn: global + bob-only, never alice-only.
        let bob = names(discover_workflows_with_profile(
            Some(home.path()),
            None,
            Some(bob_root.path()),
            false,
        ));
        assert_eq!(bob, vec!["bob-only", "global-skill"]);
    }

    /// A profile-local skill named the same as a global skill wins for its owner
    /// (highest precedence) and is tagged `WorkflowScope::Profile` (2a collision
    /// precedence).
    #[test]
    fn profile_local_wins_same_name_collision() {
        let home = tempfile::TempDir::new().unwrap();
        seed_bundle(
            &home.path().join(".openhuman").join("skills"),
            "shared-name",
        );
        let profile_root = tempfile::TempDir::new().unwrap();
        seed_bundle(profile_root.path(), "shared-name");

        let workflows = discover_workflows_with_profile(
            Some(home.path()),
            None,
            Some(profile_root.path()),
            false,
        );
        let winner = workflows
            .iter()
            .find(|w| w.name == "shared-name")
            .expect("shared-name resolved");
        // Exactly one entry for the name, and it is the profile-local copy.
        assert_eq!(
            workflows.iter().filter(|w| w.name == "shared-name").count(),
            1,
            "collision must collapse to a single winner"
        );
        assert_eq!(
            winner.scope,
            WorkflowScope::Profile,
            "profile-local skill must win the same-name collision"
        );
        // The winner resolves under the profile root, not the global one.
        let canon_profile = std::fs::canonicalize(profile_root.path()).unwrap();
        let loc = std::fs::canonicalize(winner.location.as_ref().unwrap()).unwrap();
        assert!(
            loc.starts_with(&canon_profile),
            "winning skill must live under the profile root, got {}",
            loc.display()
        );
    }

    #[test]
    fn profile_local_wins_same_runnable_id_with_different_display_name() {
        let home = tempfile::TempDir::new().unwrap();
        seed_bundle_with_name(
            &home.path().join(".openhuman").join("skills"),
            "shared-slug",
            "Global display name",
        );
        let profile_root = tempfile::TempDir::new().unwrap();
        seed_bundle_with_name(profile_root.path(), "shared-slug", "Profile display name");

        let workflows = discover_workflows_with_profile(
            Some(home.path()),
            None,
            Some(profile_root.path()),
            false,
        );
        let by_slug: Vec<_> = workflows
            .iter()
            .filter(|workflow| workflow.dir_name == "shared-slug")
            .collect();
        assert_eq!(by_slug.len(), 1, "runnable ids must be unique");
        assert_eq!(by_slug[0].scope, WorkflowScope::Profile);
        assert_eq!(by_slug[0].name, "Profile display name");
    }

    /// `WorkflowScope::Profile` outranks every global scope in the precedence
    /// ladder (the mechanism the collision test relies on).
    #[test]
    fn profile_scope_has_highest_precedence() {
        assert!(precedence(WorkflowScope::Profile) > precedence(WorkflowScope::Project));
        assert!(precedence(WorkflowScope::Profile) > precedence(WorkflowScope::User));
        assert!(precedence(WorkflowScope::Profile) > precedence(WorkflowScope::Legacy));
    }

    /// Seed a runnable bundle with a bundled resource under `references/`.
    fn seed_bundle_with_resource(root: &Path, slug: &str, resource_body: &str) {
        let dir = root.join(slug);
        std::fs::create_dir_all(dir.join("references")).unwrap();
        std::fs::write(
            dir.join("WORKFLOW.md"),
            format!("---\nname: {slug}\ndescription: {slug} desc\n---\n\n{slug} body\n"),
        )
        .unwrap();
        std::fs::write(dir.join("references").join("note.md"), resource_body).unwrap();
    }

    /// `read_workflow_resource_with_profile` (the `read_workflow_resource` tool's
    /// seam) resolves a profile's private skill resources for the owner only,
    /// resolves collisions to the profile-local copy, hides them from other
    /// profiles / the profile-less session, and leaves global-only resources
    /// readable everywhere.
    #[test]
    fn read_workflow_resource_with_profile_resolution_matrix() {
        let ws = tempfile::TempDir::new().unwrap();
        let profile_root = tempfile::TempDir::new().unwrap();
        let other_root = tempfile::TempDir::new().unwrap();

        // Global (legacy) skill + resource, private skill + resource, and a
        // collision under both.
        seed_bundle_with_resource(&ws.path().join("skills"), "resglobal7788", "GLOBAL_RES");
        seed_bundle_with_resource(profile_root.path(), "reslocal7788", "LOCAL_RES");
        seed_bundle_with_resource(&ws.path().join("skills"), "rescollide7788", "GLOBAL_RES");
        seed_bundle_with_resource(profile_root.path(), "rescollide7788", "PROFILE_RES");

        let rel = Path::new("references/note.md");
        let read = |id: &str, root: Option<&Path>| {
            read_workflow_resource_with_profile(ws.path(), id, rel, root)
        };

        // Owner reads its private skill's resource.
        assert_eq!(
            read("reslocal7788", Some(profile_root.path())).unwrap(),
            "LOCAL_RES"
        );
        // Profile-less + other profile cannot resolve the private skill at all.
        assert!(read("reslocal7788", None).is_err());
        assert!(read("reslocal7788", Some(other_root.path())).is_err());

        // Global-only resource is readable with or without a profile root.
        assert_eq!(read("resglobal7788", None).unwrap(), "GLOBAL_RES");
        assert_eq!(
            read("resglobal7788", Some(profile_root.path())).unwrap(),
            "GLOBAL_RES"
        );

        // Collision: owner reads the profile-local resource; everyone else the global.
        assert_eq!(
            read("rescollide7788", Some(profile_root.path())).unwrap(),
            "PROFILE_RES"
        );
        assert_eq!(read("rescollide7788", None).unwrap(), "GLOBAL_RES");
    }

    /// `profile_local_skill_ids` returns both runnable names and directory slugs
    /// under the profile root (the implicit-allow set the describe/read/run tools
    /// consult), and is empty for the profile-less session.
    #[test]
    fn profile_local_skill_ids_lists_only_the_profile_root() {
        let profile_root = tempfile::TempDir::new().unwrap();
        seed_bundle(profile_root.path(), "priv-a");
        seed_bundle(profile_root.path(), "priv-b");

        let ids = profile_local_skill_ids(Some(profile_root.path()));
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("priv-a"));
        assert!(ids.contains("priv-b"));

        assert!(
            profile_local_skill_ids(None).is_empty(),
            "profile-less session has no implicitly-allowed profile-local ids"
        );
    }

    #[test]
    fn profile_local_skill_ids_include_distinct_name_and_slug() {
        let profile_root = tempfile::TempDir::new().unwrap();
        seed_bundle_with_name(profile_root.path(), "mail-helper", "Inbox Assistant");

        let ids = profile_local_skill_ids(Some(profile_root.path()));
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("mail-helper"));
        assert!(ids.contains("Inbox Assistant"));
    }

    /// A `None` profile root reproduces `load_workflow_metadata` byte-for-byte —
    /// the back-compat guarantee for the profile-less session.
    #[test]
    fn none_profile_root_matches_plain_discovery() {
        let home = tempfile::TempDir::new().unwrap();
        seed_bundle(&home.path().join(".openhuman").join("skills"), "a-skill");
        let with_none = discover_workflows_with_profile(Some(home.path()), None, None, false);
        let plain = discover_workflows(Some(home.path()), None, false);
        let names: Vec<&str> = with_none.iter().map(|w| w.name.as_str()).collect();
        let plain_names: Vec<&str> = plain.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, plain_names);
    }
}
