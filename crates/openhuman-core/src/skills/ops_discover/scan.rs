//! Root-directory scanning: which on-disk roots exist per scope, walking them
//! into `Workflow` entries, and the shared multi-root scan engine that both
//! the full discovery surface and the automations-only view share.

use std::path::{Path, PathBuf};

use tinyskills::{resolve_collisions_with, CollisionPolicy, TieBreak};

use {tinyskills::Skill as Workflow, tinyskills::SkillScope as WorkflowScope};

/// Which on-disk root category a bundle was discovered under.
///
/// `Workflow` roots (`.openhuman/workflows/`) hold task *automations* authored
/// via "New workflow". `Skill` roots (`.openhuman/skills/`, `.agents/skills/`,
/// and the legacy `<workspace>/skills/`) hold capability *skills*. Both are the
/// same on-disk primitive (SKILL.md / WORKFLOW.md bundles) and the agent
/// harness loads both — but the Automations UI lists only `Workflow`-root
/// bundles (see [`super::discover_automations`]) so capability skills don't
/// masquerade as task templates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RootKind {
    Skill,
    Workflow,
}

pub(super) const ALL_ROOT_KINDS: &[RootKind] = &[RootKind::Skill, RootKind::Workflow];
pub(super) const WORKFLOW_ROOT_KINDS: &[RootKind] = &[RootKind::Workflow];

/// Shared discovery core. `kinds` selects which root categories to scan,
/// letting the full surface ([`super::discover_workflows_inner`]) and the
/// automations-only list ([`super::discover_automations`]) share collision
/// handling.
pub(super) fn discover_filtered(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
    kinds: &[RootKind],
) -> Vec<Workflow> {
    tracing::debug!(
        trusted,
        has_home = home_dir.is_some(),
        has_workspace = workspace_dir.is_some(),
        include_skills = kinds.contains(&RootKind::Skill),
        include_workflows = kinds.contains(&RootKind::Workflow),
        "[workflows] discover:enter"
    );
    // Scan order matters for collision resolution: among equal-precedence
    // scopes the last root to register a name wins (`TieBreak::LastWins`), so
    // we scan builtin first, then user, then project, then legacy.
    let mut discovered: Vec<Workflow> = Vec::new();

    // Builtin skills (`<workspace>/.openhuman/builtin-skills/`) are a skill
    // root scanned FIRST and at the lowest precedence, so every other scope
    // shadows them on a name collision. No trust marker is consulted: the
    // directory is core-managed and its contents were written from constants
    // compiled into this binary, which is a stronger provenance claim than the
    // marker makes about a project directory. See `skills::bundled`.
    if let Some(ws) = workspace_dir {
        if kinds.contains(&RootKind::Skill) {
            let root = crate::skills::bundled::builtin_root(ws);
            tracing::trace!(
                root = %root.display(),
                scope = ?WorkflowScope::Builtin,
                "[workflows] discover:branch:builtin"
            );
            discovered.extend(scan_bundled_root(&root, WorkflowScope::Builtin));
        }
    }

    if let Some(home) = home_dir {
        for (root, kind) in user_roots(home) {
            if kinds.contains(&kind) {
                tracing::trace!(
                    root = %root.display(),
                    ?kind,
                    scope = ?WorkflowScope::User,
                    "[workflows] discover:branch:user"
                );
                discovered.extend(scan_root(&root, WorkflowScope::User));
            }
        }
    }

    if let Some(agent_home) = workspace_dir.and_then(crate::skills::write_root::agent_skill_home) {
        let [skills, workflows] = crate::skills::write_root::agent_user_roots(&agent_home);
        for (root, kind) in [(skills, RootKind::Skill), (workflows, RootKind::Workflow)] {
            if kinds.contains(&kind) {
                tracing::trace!(
                    root = %root.display(),
                    ?kind,
                    scope = ?WorkflowScope::User,
                    "[workflows] discover:branch:agent"
                );
                discovered.extend(scan_root(&root, WorkflowScope::User));
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
                    discovered.extend(scan_root(&root, WorkflowScope::Project));
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
            discovered.extend(scan_root(&legacy_root, WorkflowScope::Legacy));
        }
    }

    // Cross-scope precedence and shadowing warnings are owned by tinyskills;
    // `Profile` bundles are not a filesystem scope OpenHuman scans.
    let out = resolve_collisions_with(
        discovered,
        &CollisionPolicy {
            tie_break: TieBreak::LastWins,
            excluded_scopes: vec![WorkflowScope::Profile],
            id_noun: "workflow".into(),
        },
    );
    tracing::debug!(discovered_count = out.len(), "[workflows] discover:exit");
    out
}

fn scan_bundled_root(root: &Path, scope: WorkflowScope) -> Vec<Workflow> {
    let mut out = Vec::new();
    for bundled in crate::skills::bundled::BUNDLED {
        let dir = root.join(bundled.dir_name);
        if crate::skills::bundled::is_current_materialization(&dir, bundled) {
            if let Some(mut workflow) = tinyskills::load_skill_dir(&dir, scope) {
                normalize_source_format(&mut workflow);
                out.push(workflow);
            }
        }
    }
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

pub(super) fn scan_root(root: &Path, scope: WorkflowScope) -> Vec<Workflow> {
    let mut workflows = tinyskills::scan_root(root, scope);
    for workflow in &mut workflows {
        normalize_source_format(workflow);
    }
    workflows
}

fn normalize_source_format(workflow: &mut Workflow) {
    if workflow.source_format == "agentskills" {
        workflow.source_format = "openhuman".to_string();
    }
}
