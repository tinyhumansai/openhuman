//! Workflow creation: scaffolding new SKILL.md-based skills on disk.

use serde::Deserialize;
use std::path::Path;

use tinyskills::{
    render_manifest_toml, scaffold_bundle, slugify, validate_description, validate_display_name,
    validate_manifest_inputs, BundleSpec, ScaffoldOptions,
};

use super::ops_discover::{discover_workflows_inner, is_workspace_trusted};
use {
    super::ops_types::SKILL_TOML, super::ops_types::WORKFLOW_TOML, tinyskills::Skill as Workflow,
    tinyskills::SkillScope as WorkflowScope,
};

/// One declared `[[inputs]]` entry as supplied at create time by the
/// Create-a-Workflow form. The wire shape, validation and `skill.toml`
/// rendering are owned by `tinyskills::manifest`.
pub type WorkflowCreateInputDef = tinyskills::ManifestInput;

/// Input for [`create_workflow`]. Mirrors the `skills.create` JSON-RPC payload.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CreateWorkflowParams {
    /// Human-readable name — slugified into the on-disk folder.
    pub name: String,
    /// One-line description of the procedure — what the workflow does
    /// (written into the SKILL.md frontmatter).
    pub description: String,
    /// Optional trigger/goal: *when* an agent should reach for this workflow.
    /// This is the "reason to run" a bare procedure md lacks — it merges the
    /// old agent-workflow's `when_to_use` into the unified create form. Written
    /// to the `skill.toml` `when_to_use` field; falls back to `description`
    /// when omitted.
    #[serde(default)]
    pub when_to_use: Option<String>,
    /// Where to install: `user`, `project`, or `legacy`. Defaults to `user`.
    #[serde(default)]
    pub scope: WorkflowScope,
    /// Optional SPDX license (written to frontmatter `license`).
    #[serde(default)]
    pub license: Option<String>,
    /// Optional author name (written under frontmatter `metadata.author`).
    #[serde(default)]
    pub author: Option<String>,
    /// Optional tags (written under frontmatter `metadata.tags`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional tool hints (written to frontmatter `allowed-tools`).
    #[serde(default, rename = "allowed-tools", alias = "allowed_tools")]
    pub allowed_tools: Vec<String>,
    /// Declared `[[inputs]]` for the skill. When non-empty,
    /// `create_workflow_inner` writes a sibling `skill.toml` next to the
    /// generated `SKILL.md` so the Skills Runner can render dynamic
    /// form controls for the inputs at run time.
    #[serde(default)]
    pub inputs: Vec<WorkflowCreateInputDef>,
    /// Edit mode: when `true`, an existing workflow at the resolved slug is
    /// overwritten (frontmatter + `skill.toml` rewritten) instead of rejected,
    /// and the existing `SKILL.md` body (hand-authored instructions) is
    /// preserved. Set by the `skills_update` path; `false` for create.
    #[serde(default)]
    pub overwrite: bool,
}

/// Scaffold a new SKILL.md-based skill on disk.
///
/// Writes `<scope-root>/<slug>/SKILL.md` with frontmatter derived from
/// `params` and creates empty `scripts/`, `references/`, `assets/` subdirs
/// so the author has somewhere to drop bundled resources.
///
/// Scope resolution:
/// * [`WorkflowScope::User`] → `~/.openhuman/skills/`
/// * [`WorkflowScope::Project`] → `<workspace>/.openhuman/skills/`. Requires the
///   trust marker at `<workspace>/.openhuman/trust` to be present; otherwise
///   rejected with an error.
/// * [`WorkflowScope::Legacy`] → rejected. Callers must pick one of the
///   above; the legacy `<workspace>/skills/` layout is read-only going
///   forward.
///
/// Name hardening:
/// * Slug is derived from `params.name` (lowercased, `[a-z0-9-]` only,
///   non-alphanumeric runs collapsed to a single `-`).
/// * Empty / non-alphanumeric-only names are rejected.
/// * Slug is length-bounded by `MAX_NAME_LEN`.
/// * The resolved `<scope-root>/<slug>` path is canonicalized and verified
///   to stay inside the canonical scope root (same `starts_with` guard used
///   by [`read_workflow_resource`]) to defeat `..` or absolute-path inputs.
/// * Collisions with an existing directory are rejected outright — this
///   function never overwrites.
///
/// On success the freshly created skill is re-discovered through the standard
/// pipeline and returned so callers can drop it straight into the UI list.
pub fn create_workflow(
    workspace_dir: &Path,
    params: CreateWorkflowParams,
) -> Result<Workflow, String> {
    let home = dirs::home_dir();
    create_workflow_inner(home.as_deref(), workspace_dir, params)
}

pub(crate) fn create_workflow_inner(
    home_dir: Option<&Path>,
    workspace_dir: &Path,
    mut params: CreateWorkflowParams,
) -> Result<Workflow, String> {
    tracing::debug!(
        name = %params.name,
        scope = ?params.scope,
        workspace = %workspace_dir.display(),
        "[skills] create_workflow: entry"
    );

    validate_manifest_inputs(&mut params.inputs).map_err(|e| e.to_string())?;

    let display_name = validate_display_name(&params.name).map_err(|e| e.to_string())?;
    let description = validate_description(&params.description).map_err(|e| e.to_string())?;
    let slug = slugify(display_name).map_err(|e| e.to_string())?;

    let scope_root = match params.scope {
        WorkflowScope::User => {
            crate::skills::write_root::user_workflow_root(workspace_dir, home_dir)
                .ok_or_else(|| "could not resolve user home directory".to_string())?
        }
        WorkflowScope::Project => {
            if !is_workspace_trusted(workspace_dir) {
                return Err(format!(
                    "workspace {} is not trusted; create {}/.openhuman/trust to enable project-scope workflows",
                    workspace_dir.display(),
                    workspace_dir.display(),
                ));
            }
            workspace_dir.join(".openhuman").join("workflows")
        }
        WorkflowScope::Flow => {
            // Named separately from the others because the fix differs: the
            // caller does not want a different skill scope, they want a
            // different tool.
            return Err(
                "'flow' is not a skill scope — a Flows automation is a saved graph, not a \
                 SKILL.md bundle. Use `save_workflow` / `create_workflow` to author one."
                    .to_string(),
            );
        }
        WorkflowScope::Builtin | WorkflowScope::Legacy => {
            return Err(
                "cannot create skill in legacy or builtin scope; choose 'user' or 'project'"
                    .to_string(),
            );
        }
        _ => {
            return Err(
                "cannot create skill in this scope; choose 'user' or 'project'".to_string(),
            );
        }
    };

    // Containment, create/edit preconditions, body preservation, legacy
    // SKILL.md migration and resource dirs are owned by tinyskills.
    let spec = BundleSpec {
        slug: slug.clone(),
        description: description.to_owned(),
        license: params.license.clone(),
        author: params.author.clone(),
        tags: params.tags.clone(),
        allowed_tools: params.allowed_tools.clone(),
    };
    let options = ScaffoldOptions {
        overwrite: params.overwrite,
        legacy_roots: tinyskills::legacy_scaffold_roots(home_dir, workspace_dir, params.scope),
        ..Default::default()
    };
    let scaffolded = scaffold_bundle(&scope_root, &spec, &options).map_err(|e| e.to_string())?;
    let skill_dir = scaffolded.dir;
    let workflow_md_path = scaffolded.document;

    // Emit a sibling skill.toml when the user declared `[[inputs]]` OR gave a
    // distinct `when_to_use` trigger at create time. The registry reads this
    // for the workflow's `when_to_use` (the "when to run me" signal) and to
    // render dynamic input controls. A bare workflow with neither needs no
    // skill.toml — the registry parses SKILL.md-only workflows and derives
    // `when_to_use` from the description.
    let when_to_use = params
        .when_to_use
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let workflow_toml_path = skill_dir.join(WORKFLOW_TOML);
    if !params.inputs.is_empty() || when_to_use.is_some() {
        // Distinct trigger when provided, else reuse the description so the
        // field is never empty (matches the prior behaviour).
        let workflow_toml =
            render_manifest_toml(&slug, when_to_use.unwrap_or(description), &params.inputs);
        std::fs::write(&workflow_toml_path, workflow_toml)
            .map_err(|e| format!("failed to write {}: {e}", workflow_toml_path.display()))?;
    } else if params.overwrite {
        // Edit removed all inputs + when_to_use → no manifest needed; drop any
        // stale one so the workflow reverts to a bare definition.
        let _ = std::fs::remove_file(&workflow_toml_path);
    }
    // Edit migration: retire any legacy skill.toml now that workflow.toml is
    // authoritative (avoids two manifests in the same dir).
    if params.overwrite {
        let legacy_toml = skill_dir.join(SKILL_TOML);
        if legacy_toml != workflow_toml_path && legacy_toml.exists() {
            let _ = std::fs::remove_file(&legacy_toml);
        }
    }

    tracing::info!(
        slug = %slug,
        scope = ?params.scope,
        location = %workflow_md_path.display(),
        "[skills] create_workflow: wrote SKILL.md"
    );

    let trusted = is_workspace_trusted(workspace_dir);
    let created = discover_workflows_inner(home_dir, Some(workspace_dir), trusted)
        .into_iter()
        .find(|s| s.name == slug)
        .ok_or_else(|| format!("created skill '{slug}' but failed to re-discover"))?;

    // Notify live agent sessions so they pick up the new skill in their
    // `## Installed Skills` catalogue (see `OpenHumanSessionHost::refresh_workflows`).
    crate::skills::ops_discover::invalidate_workflow_metadata_cache();
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::WorkflowsChanged {
        reason: "create".to_string(),
    });

    Ok(created)
}
