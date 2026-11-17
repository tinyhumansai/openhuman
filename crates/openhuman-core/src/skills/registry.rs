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

use crate::agent_registry::host_definition::AgentDefinition;
use tinyagents_definition::PromptSource;
use tinyskills::Skill as Workflow;

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
/// the preflight described in [`crate::skills::schemas`]'s
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
    /// [`crate::skills::runtime::spawn_workflow_run_background`].
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
    load_workflows_and_discovered(workspace_dir).0
}

/// [`load_workflows`] plus the discovered bundles the definitions
/// were built from.
///
/// Discovery re-reads and re-parses every bundle under every root, so a caller
/// that needs both the runnable definitions and the discovered metadata (scope
/// and frontmatter `name`, which `WorkflowDefinition` does not carry) takes
/// them from this one pass instead of running discovery a second time (#6166).
fn load_workflows_and_discovered(workspace_dir: &Path) -> (Vec<WorkflowDefinition>, Vec<Workflow>) {
    // Prune any legacy bundled skills an older build left behind so discover's
    // legacy scan no longer surfaces them (idempotent).
    prune_legacy_default_workflows(workspace_dir);

    let mut workflows: Vec<WorkflowDefinition> = Vec::new();

    if let Ok(builtins) = crate::agent_registry::agents::load_builtins() {
        for definition in builtins {
            workflows.push(WorkflowDefinition {
                definition,
                inputs: Vec::new(),
                github: None,
            });
        }
    }

    // Enumerate across all roots (deduped + scope-prioritised) via the same
    // discovery the create/list path uses, then load each one's definition.
    let home = super::ops_discover::discovery_home_dir();
    let trusted = super::ops_discover::is_workspace_trusted(workspace_dir);
    let discovered =
        super::ops_discover::discover_workflows(home.as_deref(), Some(workspace_dir), trusted);
    for wf in &discovered {
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
        let slug = wf.id();
        if let Some(def) = load_workflow_definition(dir, slug, &wf.description) {
            workflows.push(def);
        }
    }
    (workflows, discovered)
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
    // `read_document` is bounded and refuses symlinks.
    let md = tinyskills::read_document(&dir.join("WORKFLOW.md"))
        .or_else(|_| tinyskills::read_document(&dir.join("SKILL.md")))
        .ok()?;

    let manifest = tinyskills::read_document(&dir.join("workflow.toml"))
        .or_else(|_| tinyskills::read_document(&dir.join("skill.toml")));
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
    let (workflows, _) = load_workflows_and_discovered(workspace_dir);
    workflows
        .into_iter()
        .rev()
        .find(|workflow| workflow.definition.id == id)
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
