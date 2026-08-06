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
use crate::openhuman::skills::WorkflowScope;

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
    // Prune any legacy bundled skills an older build left behind so discover's
    // legacy scan no longer surfaces them (idempotent).
    prune_legacy_default_workflows(workspace_dir);

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

    // Enumerate across all roots (deduped + scope-prioritised) via the same
    // discovery the create/list path uses, then load each one's definition.
    let home = dirs::home_dir();
    let trusted = super::ops_discover::is_workspace_trusted(workspace_dir);
    for wf in super::ops_discover::discover_workflows_with_profile(
        home.as_deref(),
        Some(workspace_dir),
        profile_skills_root,
        trusted,
    ) {
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
    let workflows = load_workflows_with_profile(workspace_dir, profile_skills_root);
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
    let home = dirs::home_dir();
    let trusted = super::ops_discover::is_workspace_trusted(workspace_dir);
    let slug = super::ops_discover::discover_workflows_with_profile(
        home.as_deref(),
        Some(workspace_dir),
        profile_skills_root,
        trusted,
    )
    .into_iter()
    .find(|workflow| workflow.scope == WorkflowScope::Profile && workflow.name == id)
    .map(|workflow| {
        if workflow.dir_name.is_empty() {
            workflow.name
        } else {
            workflow.dir_name
        }
    })?;

    workflows
        .into_iter()
        .rev()
        .find(|workflow| workflow.definition.id == slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defs() -> Vec<WorkflowInput> {
        vec![
            WorkflowInput {
                name: "repo".into(),
                description: "owner/name".into(),
                required: true,
                kind: None,
            },
            WorkflowInput {
                name: "issue".into(),
                description: "issue #".into(),
                required: true,
                kind: Some("integer".into()),
            },
            WorkflowInput {
                name: "pr_base".into(),
                description: "base branch".into(),
                required: false,
                kind: None,
            },
        ]
    }

    #[test]
    fn missing_required_is_detected() {
        assert_eq!(
            missing_required_inputs(&defs(), &json!({"repo": "acme/web"})),
            vec!["issue".to_string()]
        );
        assert!(
            missing_required_inputs(&defs(), &json!({"repo": "acme/web", "issue": 42})).is_empty()
        );
        // null counts as missing
        assert_eq!(
            missing_required_inputs(&defs(), &json!({"repo": "acme/web", "issue": null})),
            vec!["issue".to_string()]
        );
    }

    #[test]
    fn renders_inputs_block_with_values_and_gaps() {
        let b = render_inputs_block(&defs(), &json!({"repo": "acme/web", "issue": 42}));
        assert!(b.starts_with("## Inputs"));
        assert!(b.contains("**repo**: acme/web"));
        assert!(b.contains("**issue**: 42"));
        assert!(b.contains("**pr_base**: (not provided)"));
        assert!(render_inputs_block(&[], &json!({})).is_empty());
    }

    #[test]
    fn skill_input_parses_type_alias() {
        let i: WorkflowInput = serde_json::from_value(json!({
            "name": "issue", "description": "issue #", "required": true, "type": "integer"
        }))
        .unwrap();
        assert_eq!(i.kind.as_deref(), Some("integer"));
        assert!(i.required);
    }

    /// Seed a runnable WORKFLOW.md bundle under `root/slug/` with a distinct
    /// body marker so the resolved definition can be traced back to its source.
    fn seed_runnable(root: &std::path::Path, slug: &str, body_marker: &str) {
        seed_runnable_with_name(root, slug, slug, body_marker);
    }

    fn seed_runnable_with_name(root: &std::path::Path, slug: &str, name: &str, body_marker: &str) {
        let dir = root.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("WORKFLOW.md"),
            format!("---\nname: {name}\ndescription: {name} desc\n---\n\n{body_marker}\n"),
        )
        .unwrap();
    }

    fn resolved_body(def: &WorkflowDefinition) -> String {
        match &def.definition.system_prompt {
            PromptSource::Inline(p) => p.clone(),
            other => panic!("expected inline prompt, got {other:?}"),
        }
    }

    /// The resolution seam behind `describe_workflow` / `run_workflow`
    /// (`get_workflow_with_profile`) resolves a profile's private skills for the
    /// owner only, resolves collisions to the profile-local copy, keeps
    /// profile-local skills invisible to other profiles / the profile-less
    /// session, and leaves global-only skills resolvable everywhere.
    #[test]
    fn get_workflow_with_profile_resolution_matrix() {
        // Unique-ish ids so a developer's real ~/.openhuman/skills can't collide.
        let ws = tempfile::TempDir::new().unwrap();
        let profile_root = tempfile::TempDir::new().unwrap();
        let other_root = tempfile::TempDir::new().unwrap(); // a different profile

        // A global skill under the legacy `<ws>/skills/` root (no trust marker
        // needed), and a private skill under the profile root.
        seed_runnable(&ws.path().join("skills"), "zzglobalonly7788", "GLOBAL_BODY");
        seed_runnable(profile_root.path(), "zzlocalonly7788", "LOCAL_BODY");
        // Collision: same id in both the global legacy root and the profile root.
        seed_runnable(&ws.path().join("skills"), "zzcollide7788", "GLOBAL_COLLIDE");
        seed_runnable(profile_root.path(), "zzcollide7788", "PROFILE_COLLIDE");

        let get = |id: &str, root: Option<&std::path::Path>| {
            get_workflow_with_profile(ws.path(), id, root)
        };

        // Owner resolves its profile-local skill.
        assert!(
            get("zzlocalonly7788", Some(profile_root.path())).is_some(),
            "owner must resolve its profile-local skill"
        );
        // Profile-less session and a different profile cannot resolve it.
        assert!(
            get("zzlocalonly7788", None).is_none(),
            "profile-less session must not resolve a profile-local skill"
        );
        assert!(
            get("zzlocalonly7788", Some(other_root.path())).is_none(),
            "a different profile must not resolve another profile's private skill"
        );

        // Global-only skill resolves everywhere (with/without a profile root).
        assert!(get("zzglobalonly7788", None).is_some());
        assert!(get("zzglobalonly7788", Some(profile_root.path())).is_some());
        assert!(get("zzglobalonly7788", Some(other_root.path())).is_some());

        // Collision: the owner resolves the profile-local copy; everyone else
        // resolves the global copy.
        assert_eq!(
            resolved_body(&get("zzcollide7788", Some(profile_root.path())).unwrap()),
            "---\nname: zzcollide7788\ndescription: zzcollide7788 desc\n---\n\nPROFILE_COLLIDE\n",
            "owner must resolve the profile-local copy on collision"
        );
        assert_eq!(
            resolved_body(&get("zzcollide7788", None).unwrap()),
            "---\nname: zzcollide7788\ndescription: zzcollide7788 desc\n---\n\nGLOBAL_COLLIDE\n",
            "profile-less session resolves the global copy on collision"
        );
    }

    #[test]
    fn get_profile_workflow_resolves_distinct_display_name() {
        let ws = tempfile::TempDir::new().unwrap();
        let profile_root = tempfile::TempDir::new().unwrap();
        seed_runnable_with_name(
            profile_root.path(),
            "mail-helper",
            "Inbox Assistant",
            "PROFILE_NAME_BODY",
        );

        let resolved =
            get_workflow_with_profile(ws.path(), "Inbox Assistant", Some(profile_root.path()))
                .expect("display name must resolve for the owning profile");
        assert_eq!(resolved.definition.id, "mail-helper");
        assert!(resolved_body(&resolved).contains("PROFILE_NAME_BODY"));
        assert!(get_workflow_with_profile(ws.path(), "Inbox Assistant", None).is_none());
    }

    #[test]
    fn profile_workflow_exact_id_overrides_builtin() {
        let ws = tempfile::TempDir::new().unwrap();
        let profile_root = tempfile::TempDir::new().unwrap();
        seed_runnable(profile_root.path(), "critic", "PROFILE_CRITIC_BODY");

        let resolved = get_workflow_with_profile(ws.path(), "critic", Some(profile_root.path()))
            .expect("profile critic resolves");
        assert!(resolved_body(&resolved).contains("PROFILE_CRITIC_BODY"));
    }

    #[test]
    fn load_skills_reads_runtime_skill_prompt_and_inputs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sd = tmp.path().join("skills").join("issue-crusher");
        std::fs::create_dir_all(&sd).unwrap();
        std::fs::write(
            sd.join("skill.toml"),
            "id = \"issue-crusher\"\nwhen_to_use = \"fix a github issue\"\n\
             [[inputs]]\nname = \"repo\"\ndescription = \"owner/name\"\nrequired = true\n\
             [[inputs]]\nname = \"issue\"\ndescription = \"issue #\"\nrequired = true\ntype = \"integer\"\n",
        )
        .unwrap();
        std::fs::write(sd.join("SKILL.md"), "# Issue Crusher\nFix it.").unwrap();

        let skills = load_workflows(tmp.path());
        let s = skills
            .iter()
            .find(|s| s.definition.id == "issue-crusher")
            .expect("runtime skill loaded");
        assert_eq!(s.inputs.len(), 2);
        assert_eq!(s.inputs[1].kind.as_deref(), Some("integer"));
        match &s.definition.system_prompt {
            PromptSource::Inline(p) => assert!(p.contains("Fix it.")),
            other => panic!("expected inline prompt, got {other:?}"),
        }
    }

    #[test]
    fn skill_md_only_install_resolves_by_dir_slug_not_frontmatter_name() {
        // Regression (#3987 codex review): a SKILL.md-only install whose
        // frontmatter `name` differs from its install slug must resolve via the
        // dir slug — the id surfaced in the list summary / orchestrator prompt /
        // uninstall — not the frontmatter name. Before the fix, `definition.id`
        // was built from `wf.name` ("My Cool Workflow"), so `get_workflow`
        // (keyed on the slug) returned None → "unknown skill".
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("skills").join("my-cool-workflow");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: My Cool Workflow\ndescription: does cool things\n---\n\n# Body\n",
        )
        .unwrap();

        let resolved = get_workflow(tmp.path(), "my-cool-workflow")
            .expect("SKILL.md-only install must resolve by its dir slug");
        assert_eq!(resolved.definition.id, "my-cool-workflow");
        // And NOT by the frontmatter name.
        assert!(
            get_workflow(tmp.path(), "My Cool Workflow").is_none(),
            "frontmatter name must not be the runnable id"
        );
    }

    #[test]
    fn prune_removes_legacy_bundled_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills = tmp.path().join("skills");
        // A legacy bundled id + a user-authored workflow that must survive.
        for id in ["github-issue-crusher", "my-workflow"] {
            std::fs::create_dir_all(skills.join(id)).unwrap();
            std::fs::write(skills.join(id).join("SKILL.md"), "# x").unwrap();
        }
        prune_legacy_default_workflows(tmp.path());
        assert!(
            !skills.join("github-issue-crusher").exists(),
            "legacy bundled id should be pruned"
        );
        assert!(
            skills.join("my-workflow").exists(),
            "user-authored workflow must be left untouched"
        );
    }

    #[test]
    fn skill_github_config_defaults_when_absent() {
        // No [github] block in skill.toml → `github` deserialises to None,
        // which the preflight reads as "gate disabled, skip silently".
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n";
        let parsed: WorkflowDefinition = toml::from_str(toml).expect("parse");
        assert!(parsed.github.is_none(), "no [github] block ⇒ None");
    }

    #[test]
    fn skill_github_config_parses_full_block() {
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n\
                    [github]\nrequired = true\nidentity_match = \"strict\"\n";
        let parsed: WorkflowDefinition = toml::from_str(toml).expect("parse");
        let gh = parsed.github.expect("github block present");
        assert!(gh.required);
        assert_eq!(gh.identity_match, IdentityMatch::Strict);
    }

    #[test]
    fn skill_github_config_required_defaults_to_false() {
        // Block present but required not set ⇒ required = false (default).
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n\
                    [github]\nidentity_match = \"any\"\n";
        let parsed: WorkflowDefinition = toml::from_str(toml).expect("parse");
        let gh = parsed.github.expect("github block present");
        assert!(!gh.required, "required defaults to false");
        assert_eq!(gh.identity_match, IdentityMatch::Any);
    }

    #[test]
    fn skill_github_config_identity_match_defaults_to_strict() {
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n\
                    [github]\nrequired = true\n";
        let parsed: WorkflowDefinition = toml::from_str(toml).expect("parse");
        let gh = parsed.github.expect("github block present");
        assert_eq!(
            gh.identity_match,
            IdentityMatch::Strict,
            "default is Strict"
        );
    }

    #[test]
    fn skill_github_config_accepts_all_identity_match_variants() {
        for (variant, expected) in [
            ("strict", IdentityMatch::Strict),
            ("any", IdentityMatch::Any),
            ("none", IdentityMatch::None),
        ] {
            let toml = format!(
                "id = \"x\"\nwhen_to_use = \"y\"\n\
                 [github]\nrequired = true\nidentity_match = \"{variant}\"\n"
            );
            let parsed: WorkflowDefinition = toml::from_str(&toml).expect("parse");
            assert_eq!(
                parsed.github.expect("github block present").identity_match,
                expected,
                "variant {variant} → {expected:?}",
            );
        }
    }

    #[test]
    fn skill_github_config_serializes_lowercase() {
        let gh = WorkflowGithubConfig {
            required: true,
            identity_match: IdentityMatch::Strict,
        };
        let s = toml::to_string(&gh).expect("serialize");
        assert!(s.contains("required = true"));
        assert!(
            s.contains("identity_match = \"strict\""),
            "lowercase serialization: got {s}"
        );
    }
}
