//! Workflow registry types: a **skill** is an [`AgentDefinition`] plus declared
//! `[[inputs]]`. The agent fields (`id`, `system_prompt`, `tools`,
//! `max_iterations`, `sandbox_mode`, …) are flattened in from the same
//! `skill.toml`, so a skill is just a runnable agent that also advertises the
//! inputs it needs. Schema lives here; values are supplied at `skill_run` time
//! and rendered into the prompt (see [`render_inputs_block`]).
//!
//! This keeps [`AgentDefinition`] untouched (no widespread struct-literal
//! churn) — inputs ride at the skill layer via `#[serde(flatten)]`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
use crate::openhuman::skills::{Workflow, WorkflowScope};

/// One declared input — a parameter the skill needs, with a human description.
/// `required` inputs must be supplied at run time; `kind` is an optional type
/// hint (`"string"`, `"integer"`, …) for the UI / validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

/// How strictly the [`WorkflowGithubConfig`] preflight gate should compare
/// the Composio-connected GitHub identity with the local `git config
/// user.name`. Default: [`IdentityMatch::Strict`].
///
/// | Variant | Behaviour at preflight |
/// |---------|------------------------|
/// | `Strict` | The Composio-connected GitHub username MUST equal `git config user.name` (case-insensitive after trimming). Mismatch → gate fail. |
/// | `Any`    | Both must exist (Composio github connection AND local git identity) but they don't have to match. |
/// | `None`   | Skip the identity comparison entirely — only assert both subsystems are reachable. |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityMatch {
    #[default]
    Strict,
    Any,
    None,
}

/// `[github]` block in `skill.toml`. Optional; absent ⇒ no GitHub
/// preflight gate runs for this skill. Present + `required = true` ⇒
/// the preflight described in [`crate::openhuman::skills::schemas`]'s
/// `preflight_github_gate` runs before the orchestrator boots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorkflowGithubConfig {
    /// When true, the gate runs. When false (default), the gate is
    /// skipped even if other fields are populated — the gate is opt-in
    /// per skill.
    #[serde(default)]
    pub required: bool,
    /// How strictly to compare the Composio GitHub identity against
    /// local `git config user.name`. See [`IdentityMatch`].
    #[serde(default)]
    pub identity_match: IdentityMatch,
}

/// A skill = an agent definition + its declared inputs (parsed from `skill.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowDefinition {
    #[serde(flatten)]
    pub definition: AgentDefinition,
    #[serde(default)]
    pub inputs: Vec<WorkflowInput>,
    /// Optional GitHub preflight gate. When `Some(..)` with
    /// `required = true`, the preflight runs before the orchestrator
    /// boots — see
    /// [`crate::openhuman::skills::runtime::spawn_workflow_run_background`].
    #[serde(default)]
    pub github: Option<WorkflowGithubConfig>,
}

/// Names of `required` inputs that are absent or null in `provided`. Empty ⇒ OK.
pub fn missing_required_inputs(
    defs: &[WorkflowInput],
    provided: &serde_json::Value,
) -> Vec<String> {
    defs.iter()
        .filter(|d| d.required)
        .filter(|d| provided.get(&d.name).map(|v| v.is_null()).unwrap_or(true))
        .map(|d| d.name.clone())
        .collect()
}

/// Render the resolved inputs as an `## Inputs` prompt block injected alongside
/// the skill's `SKILL.md`. Empty string when the skill declares no inputs.
pub fn render_inputs_block(defs: &[WorkflowInput], provided: &serde_json::Value) -> String {
    if defs.is_empty() {
        return String::new();
    }
    let mut lines = vec!["## Inputs".to_string()];
    for d in defs {
        let shown = match provided.get(&d.name) {
            None | Some(serde_json::Value::Null) => "(not provided)".to_string(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        };
        lines.push(format!("- **{}**: {}", d.name, shown));
    }
    lines.join("\n")
}

/// Legacy bundled skills that shipped with older builds and were removed in the
/// workflows-unify refactor (the old `dev-workflow` plus the
/// `github-issue-crusher` / `pr-review-shepherd` runner skills). OpenHuman no
/// longer ships any bundled defaults; these ids are pruned from upgraded
/// workspaces so they stop surfacing in the Workflows tab.
const LEGACY_BUNDLED_WORKFLOW_IDS: &[&str] =
    &["dev-workflow", "github-issue-crusher", "pr-review-shepherd"];

/// Remove the legacy bundled skill dirs an older build seeded into
/// `<workspace>/skills/<id>/`. Bounded to [`LEGACY_BUNDLED_WORKFLOW_IDS`] so
/// user-authored workflows are never touched; idempotent (no-op once gone).
pub fn prune_legacy_default_workflows(workspace_dir: &Path) {
    let base = workspace_dir.join("skills");
    for id in LEGACY_BUNDLED_WORKFLOW_IDS {
        let dir = base.join(id);
        if !dir.exists() {
            continue;
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => log::info!(
                "[workflows] pruned legacy bundled skill '{id}' from {}",
                dir.display()
            ),
            Err(e) => log::warn!("[workflows] prune legacy skill '{id}' failed: {e}"),
        }
    }
}

/// Load the runnable workflow registry: compile-time builtins (no declared
/// inputs) + every workflow `discover_workflows` surfaces — user
/// (`~/.openhuman/skills`), project (`<ws>/.openhuman/skills`, trusted), and
/// legacy (`<ws>/skills`) — loaded into a runnable [`WorkflowDefinition`].
///
/// This is the unification fix: the RUN path now reads the SAME roots the
/// create/list path writes to, so a workflow authored on the Intelligence tab
/// (which lands in `.openhuman/skills`) is runnable, not just listable.
/// Previously this scanned only `<ws>/skills`, so `get_workflow` (and thus
/// `run_workflow`) returned "unknown workflow" for anything created via the UI.
///
/// Per dir: `skill.toml` (id / `when_to_use` / `[[inputs]]` / `[github]`)
/// + the `SKILL.md` body as the inline system prompt.
///
/// Without `skill.toml`, a synthesized SKILL.md-only definition means a bare workflow is
/// still runnable. A bad `skill.toml` falls back to the SKILL.md-only form.
pub fn load_workflows(workspace_dir: &Path) -> Vec<WorkflowDefinition> {
    load_workflows_with_profile(workspace_dir, None)
}

/// Like [`load_workflows`], but additionally resolves the active profile's
/// private skills (`<workspace>/personalities/<id>/skills/`) when
/// `profile_skills_root` is supplied.
///
/// The profile root is threaded straight into
/// [`super::ops_discover::discover_workflows_with_profile`], so profile-local
/// skills become runnable/describable for their owner and win same-name
/// collisions against global skills (via [`WorkflowScope::Profile`] precedence).
/// `None` reproduces [`load_workflows`] byte-for-byte — other profiles and the
/// profile-less session never see these skills. No global registry state is
/// mutated, so concurrent sessions under different profiles stay isolated.
pub fn load_workflows_with_profile(
    workspace_dir: &Path,
    profile_skills_root: Option<&Path>,
) -> Vec<WorkflowDefinition> {
    definitions_from_discovered(&discover_all(workspace_dir, profile_skills_root))
}

/// Prune legacy bundled skills, then enumerate every installed skill across all
/// roots (deduped + scope-prioritised) via the same discovery the create/list
/// path uses. Returns the lightweight discovered entries **without** parsing
/// each one's definition — callers that need only a slug/name/scope (e.g. the
/// display-name fallback in [`get_workflow_with_profile`]) can reuse this list
/// instead of walking the roots again.
///
/// The prune runs **before** discovery so its legacy scan no longer surfaces
/// the pruned skills (idempotent).
fn discover_all(workspace_dir: &Path, profile_skills_root: Option<&Path>) -> Vec<Workflow> {
    prune_legacy_default_workflows(workspace_dir);
    let home = dirs::home_dir();
    let trusted = super::ops_discover::is_workspace_trusted(workspace_dir);
    super::ops_discover::discover_workflows_with_profile(
        home.as_deref(),
        Some(workspace_dir),
        profile_skills_root,
        trusted,
    )
}

/// Parse a [`WorkflowDefinition`] for each already-discovered skill, prepended
/// by the built-in agents. Split out from [`load_workflows_with_profile`] so a
/// single [`discover_all`] walk can feed both the parsed list and a slug/name
/// lookup without re-discovering.
fn definitions_from_discovered(discovered: &[Workflow]) -> Vec<WorkflowDefinition> {
    let mut workflows: Vec<WorkflowDefinition> = Vec::new();

    if let Ok(builtins) = crate::openhuman::agent::registry::agents::load_builtins() {
        for definition in builtins {
            workflows.push(WorkflowDefinition {
                definition,
                inputs: Vec::new(),
                github: None,
            });
        }
    }

    for wf in discovered {
        let Some(skill_md) = wf.location.as_ref() else {
            continue;
        };
        let Some(dir) = skill_md.parent() else {
            continue;
        };
        // Build the runnable id from the on-disk slug (`dir_name`) so it matches
        // the `WorkflowSummary.id` shown in lists, the id the orchestrator prompt
        // tells the agent to run, and the slug uninstall resolves against — all
        // of which key on `dir_name`. A SKILL.md-only install whose frontmatter
        // `name` differs from its install slug (e.g. `name: My Cool Workflow` in
        // `my-cool-workflow/`) would otherwise build `definition.id` from the
        // name and be unresolvable by `skills_describe` / `skills_run`
        // ("unknown skill"). Falls back to `name` for legacy `Workflow` values
        // that predate `dir_name`. (#3987 codex review.)
        let slug = if wf.dir_name.is_empty() {
            wf.name.as_str()
        } else {
            wf.dir_name.as_str()
        };
        if let Some(def) = load_workflow_definition(dir, slug, &wf.description) {
            workflows.push(def);
        }
    }
    workflows
}

/// Build a runnable [`WorkflowDefinition`] from a single workflow directory.
/// Prefers `skill.toml`; falls back to a SKILL.md-only definition (id = the
/// discovered slug, `when_to_use` = the frontmatter description) so a workflow
/// with no `skill.toml` is still runnable. Returns `None` if `SKILL.md` is
/// unreadable.
fn load_workflow_definition(
    dir: &Path,
    slug: &str,
    description: &str,
) -> Option<WorkflowDefinition> {
    // WORKFLOW.md / workflow.toml are current; SKILL.md / skill.toml are read
    // for back-compat with workflows authored before the rename.
    let md = std::fs::read_to_string(dir.join("WORKFLOW.md"))
        .or_else(|_| std::fs::read_to_string(dir.join("SKILL.md")))
        .ok()?;

    let manifest = std::fs::read_to_string(dir.join("workflow.toml"))
        .or_else(|_| std::fs::read_to_string(dir.join("skill.toml")));
    if let Ok(toml_str) = manifest {
        match toml::from_str::<WorkflowDefinition>(&toml_str) {
            Ok(mut def) => {
                def.definition.system_prompt = PromptSource::Inline(md);
                return Some(def);
            }
            Err(e) => {
                log::warn!(
                    "[workflows] {}: bad workflow.toml ({e}); falling back to WORKFLOW.md-only",
                    dir.display()
                );
            }
        }
    }

    // SKILL.md-only: synthesize a minimal runnable definition. Build the
    // AgentDefinition through serde (only `id` + `when_to_use` lack defaults)
    // so the rest of its fields take their normal defaults.
    let mut table = toml::map::Map::new();
    table.insert("id".to_string(), toml::Value::String(slug.to_string()));
    table.insert(
        "when_to_use".to_string(),
        toml::Value::String(description.to_string()),
    );
    let mut def: WorkflowDefinition = toml::Value::Table(table).try_into().ok()?;
    def.definition.system_prompt = PromptSource::Inline(md);
    Some(def)
}

/// Look up one skill by id across the registry.
pub fn get_workflow(workspace_dir: &Path, id: &str) -> Option<WorkflowDefinition> {
    get_workflow_with_profile(workspace_dir, id, None)
}

/// Like [`get_workflow`], but resolves the active profile's private skills too
/// (`<workspace>/personalities/<id>/skills/`) when `profile_skills_root` is
/// supplied. This is the resolution seam behind `describe_workflow` /
/// `run_workflow`: a profile-local skill is runnable/describable for its owner
/// and wins same-name collisions; `None` is byte-identical to [`get_workflow`].
pub fn get_workflow_with_profile(
    workspace_dir: &Path,
    id: &str,
    profile_skills_root: Option<&Path>,
) -> Option<WorkflowDefinition> {
    // Discover once; both the parsed-definition lookup and the display-name
    // fallback below derive from this single walk. Previously each call
    // discovered the roots twice — once inside `load_workflows_with_profile`
    // and again for the name fallback.
    let discovered = discover_all(workspace_dir, profile_skills_root);
    let workflows = definitions_from_discovered(&discovered);

    // Built-ins are prepended and discovered workflows follow them. Search in
    // reverse so the scope-resolved discovered entry (profile wins over global)
    // also wins over a built-in with the same runnable id.
    if let Some(exact) = workflows.iter().rev().find(|s| s.definition.id == id) {
        return Some(exact.clone());
    }

    // Profile lists advertise the frontmatter display name as well as the
    // directory slug. Resolve that name back to the canonical runnable slug so
    // a private workflow admitted by the profile-local allow set can actually
    // be described and run. Keep the legacy profile-less lookup id-only: global
    // display names have never been runnable ids and may collide with builtins.
    // Reuse the discovery above rather than walking the roots a second time.
    let slug = discovered
        .iter()
        .find(|workflow| workflow.scope == WorkflowScope::Profile && workflow.name == id)
        .map(|workflow| {
            if workflow.dir_name.is_empty() {
                workflow.name.clone()
            } else {
                workflow.dir_name.clone()
            }
        })?;

    workflows
        .into_iter()
        .rev()
        .find(|workflow| workflow.definition.id == slug)
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
