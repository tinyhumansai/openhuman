//! Skill registry types: a **skill** is an [`AgentDefinition`] plus declared
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

/// One declared input — a parameter the skill needs, with a human description.
/// `required` inputs must be supplied at run time; `kind` is an optional type
/// hint (`"string"`, `"integer"`, …) for the UI / validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

/// How strictly the [`SkillGithubConfig`] preflight gate should compare
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillGithubConfig {
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

impl Default for SkillGithubConfig {
    fn default() -> Self {
        Self {
            required: false,
            identity_match: IdentityMatch::default(),
        }
    }
}

/// A skill = an agent definition + its declared inputs (parsed from `skill.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct SkillDefinition {
    #[serde(flatten)]
    pub definition: AgentDefinition,
    #[serde(default)]
    pub inputs: Vec<SkillInput>,
    /// Optional GitHub preflight gate. When `Some(..)` with
    /// `required = true`, the preflight runs before the orchestrator
    /// boots — see
    /// [`crate::openhuman::skills::schemas::spawn_skill_run_background`].
    #[serde(default)]
    pub github: Option<SkillGithubConfig>,
}

/// Names of `required` inputs that are absent or null in `provided`. Empty ⇒ OK.
pub fn missing_required_inputs(defs: &[SkillInput], provided: &serde_json::Value) -> Vec<String> {
    defs.iter()
        .filter(|d| d.required)
        .filter(|d| provided.get(&d.name).map(|v| v.is_null()).unwrap_or(true))
        .map(|d| d.name.clone())
        .collect()
}

/// Render the resolved inputs as an `## Inputs` prompt block injected alongside
/// the skill's `SKILL.md`. Empty string when the skill declares no inputs.
pub fn render_inputs_block(defs: &[SkillInput], provided: &serde_json::Value) -> String {
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

/// Default skills shipped *with* OpenHuman — bundled into the binary and
/// materialised into `<workspace>/skills/<id>/` on first load. Each entry is
/// `(id, skill.toml, SKILL.md)`.
const DEFAULT_SKILLS: &[(&str, &str, &str)] = &[
    (
        "github-issue-crusher",
        include_str!("defaults/github-issue-crusher/skill.toml"),
        include_str!("defaults/github-issue-crusher/SKILL.md"),
    ),
    // Phase-6 companion to github-issue-crusher: takes a single open PR and
    // iterates check → fix → push → re-check until both gates close (CI
    // green AND every actionable reviewer/bot comment addressed), surfaces a
    // real blocker, or notices the PR was merged / closed.
    (
        "pr-review-shepherd",
        include_str!("defaults/pr-review-shepherd/skill.toml"),
        include_str!("defaults/pr-review-shepherd/SKILL.md"),
    ),
    // Cron-friendly autonomous-developer skill: pick an issue assigned to
    // the user on the upstream repo and ship a PR. Designed to be wired
    // behind the DevWorkflowPanel + cron schedule (#2802) for unattended
    // recurring runs. Distinct from github-issue-crusher in that the issue
    // number is *picked* rather than passed in.
    (
        "dev-workflow",
        include_str!("defaults/dev-workflow/skill.toml"),
        include_str!("defaults/dev-workflow/SKILL.md"),
    ),
];

/// Seed the bundled [`DEFAULT_SKILLS`] into `<workspace>/skills/<id>/` when
/// absent. Idempotent and non-destructive: an existing `skill.toml` (already
/// seeded, or user-edited) is left untouched, so a default can be customised or
/// removed. This is what makes a default skill "come with the system" — every
/// workspace gets it without a manual drop.
pub fn seed_default_skills(workspace_dir: &Path) {
    let base = workspace_dir.join("skills");
    for (id, skill_toml, skill_md) in DEFAULT_SKILLS {
        let dir = base.join(id);
        if dir.join("skill.toml").exists() {
            continue; // already present — never clobber
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("[skills] seed {id}: mkdir failed: {e}");
            continue;
        }
        let _ = std::fs::write(dir.join("skill.toml"), skill_toml);
        let _ = std::fs::write(dir.join("SKILL.md"), skill_md);
        log::info!(
            "[skills] seeded default skill '{id}' into {}",
            dir.display()
        );
    }
}

/// Load the skill registry: bundled defaults (seeded into the workspace) +
/// compile-time builtins (no declared inputs) + runtime skills under
/// `<workspace>/skills/<id>/{skill.toml, SKILL.md}`. A skill's `SKILL.md`, when
/// present, becomes its inline system prompt. A bad `skill.toml` is skipped
/// with a warning, not fatal.
pub fn load_skills(workspace_dir: &Path) -> Vec<SkillDefinition> {
    // Materialise the bundled defaults (idempotent) so they're always present
    // and user-editable in the workspace, then picked up by the scan below.
    seed_default_skills(workspace_dir);

    let mut skills: Vec<SkillDefinition> = Vec::new();

    if let Ok(builtins) = crate::openhuman::agent::agents::load_builtins() {
        for definition in builtins {
            skills.push(SkillDefinition {
                definition,
                inputs: Vec::new(),
                github: None,
            });
        }
    }

    let dir = workspace_dir.join("skills");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let sd = entry.path();
            if !sd.is_dir() {
                continue;
            }
            let toml_path = sd.join("skill.toml");
            let Ok(toml_str) = std::fs::read_to_string(&toml_path) else {
                continue;
            };
            let mut skill: SkillDefinition = match toml::from_str(&toml_str) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[skills] skipping {}: {e}", toml_path.display());
                    continue;
                }
            };
            if let Ok(md) = std::fs::read_to_string(sd.join("SKILL.md")) {
                skill.definition.system_prompt = PromptSource::Inline(md);
            }
            skills.push(skill);
        }
    }
    skills
}

/// Look up one skill by id across the registry.
pub fn get_skill(workspace_dir: &Path, id: &str) -> Option<SkillDefinition> {
    load_skills(workspace_dir)
        .into_iter()
        .find(|s| s.definition.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defs() -> Vec<SkillInput> {
        vec![
            SkillInput {
                name: "repo".into(),
                description: "owner/name".into(),
                required: true,
                kind: None,
            },
            SkillInput {
                name: "issue".into(),
                description: "issue #".into(),
                required: true,
                kind: Some("integer".into()),
            },
            SkillInput {
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
        let i: SkillInput = serde_json::from_value(json!({
            "name": "issue", "description": "issue #", "required": true, "type": "integer"
        }))
        .unwrap();
        assert_eq!(i.kind.as_deref(), Some("integer"));
        assert!(i.required);
    }

    #[test]
    fn load_skills_reads_runtime_skill_prompt_and_inputs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sd = tmp.path().join("skills").join("github-issue-crusher");
        std::fs::create_dir_all(&sd).unwrap();
        std::fs::write(
            sd.join("skill.toml"),
            "id = \"github-issue-crusher\"\nwhen_to_use = \"fix a github issue\"\n\
             [[inputs]]\nname = \"repo\"\ndescription = \"owner/name\"\nrequired = true\n\
             [[inputs]]\nname = \"issue\"\ndescription = \"issue #\"\nrequired = true\ntype = \"integer\"\n",
        )
        .unwrap();
        std::fs::write(sd.join("SKILL.md"), "# Issue Crusher\nFix it.").unwrap();

        let skills = load_skills(tmp.path());
        let s = skills
            .iter()
            .find(|s| s.definition.id == "github-issue-crusher")
            .expect("runtime skill loaded");
        assert_eq!(s.inputs.len(), 2);
        assert_eq!(s.inputs[1].kind.as_deref(), Some("integer"));
        match &s.definition.system_prompt {
            PromptSource::Inline(p) => assert!(p.contains("Fix it.")),
            other => panic!("expected inline prompt, got {other:?}"),
        }
    }

    #[test]
    fn default_skills_seed_into_empty_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Fresh workspace, nothing pre-written: the bundled default must appear.
        let skills = load_skills(tmp.path());
        let s = skills
            .iter()
            .find(|s| s.definition.id == "github-issue-crusher")
            .expect("bundled default seeded + loaded");
        assert_eq!(s.inputs.len(), 4, "repo + issue + fork + pr_base");
        assert_eq!(s.inputs[0].name, "repo");
        assert!(s.inputs[0].required);
        assert_eq!(
            s.inputs[1].kind.as_deref(),
            Some("integer"),
            "issue is integer"
        );
        assert_eq!(s.inputs[2].name, "fork");
        assert!(!s.inputs[2].required, "fork is optional");
        assert!(!s.inputs[3].required, "pr_base is optional");
        match &s.definition.system_prompt {
            PromptSource::Inline(p) => assert!(p.contains("GitHub Issue Crusher")),
            other => panic!("expected inline prompt, got {other:?}"),
        }
        // Materialised on disk (user-editable), and re-seeding is non-destructive.
        let toml = tmp.path().join("skills/github-issue-crusher/skill.toml");
        assert!(toml.exists());
        std::fs::write(
            &toml,
            "id = \"github-issue-crusher\"\nwhen_to_use = \"edited\"\n",
        )
        .unwrap();
        seed_default_skills(tmp.path());
        assert!(
            std::fs::read_to_string(&toml).unwrap().contains("edited"),
            "existing skill.toml must not be clobbered"
        );
    }

    #[test]
    fn skill_github_config_defaults_when_absent() {
        // No [github] block in skill.toml → `github` deserialises to None,
        // which the preflight reads as "gate disabled, skip silently".
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n";
        let parsed: SkillDefinition = toml::from_str(toml).expect("parse");
        assert!(parsed.github.is_none(), "no [github] block ⇒ None");
    }

    #[test]
    fn skill_github_config_parses_full_block() {
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n\
                    [github]\nrequired = true\nidentity_match = \"strict\"\n";
        let parsed: SkillDefinition = toml::from_str(toml).expect("parse");
        let gh = parsed.github.expect("github block present");
        assert!(gh.required);
        assert_eq!(gh.identity_match, IdentityMatch::Strict);
    }

    #[test]
    fn skill_github_config_required_defaults_to_false() {
        // Block present but required not set ⇒ required = false (default).
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n\
                    [github]\nidentity_match = \"any\"\n";
        let parsed: SkillDefinition = toml::from_str(toml).expect("parse");
        let gh = parsed.github.expect("github block present");
        assert!(!gh.required, "required defaults to false");
        assert_eq!(gh.identity_match, IdentityMatch::Any);
    }

    #[test]
    fn skill_github_config_identity_match_defaults_to_strict() {
        let toml = "id = \"x\"\nwhen_to_use = \"y\"\n\
                    [github]\nrequired = true\n";
        let parsed: SkillDefinition = toml::from_str(toml).expect("parse");
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
            let parsed: SkillDefinition = toml::from_str(&toml).expect("parse");
            assert_eq!(
                parsed.github.expect("github block present").identity_match,
                expected,
                "variant {variant} → {expected:?}",
            );
        }
    }

    #[test]
    fn skill_github_config_serializes_lowercase() {
        let gh = SkillGithubConfig {
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

    #[test]
    fn dev_workflow_default_skill_seeds_and_loads() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills = load_skills(tmp.path());
        let s = skills
            .iter()
            .find(|s| s.definition.id == "dev-workflow")
            .expect("dev-workflow bundled default seeded + loaded");
        assert_eq!(
            s.inputs.len(),
            4,
            "repo + upstream + target_branch + fork_owner"
        );
        assert_eq!(s.inputs[0].name, "repo");
        assert_eq!(s.inputs[1].name, "upstream");
        assert_eq!(s.inputs[2].name, "target_branch");
        assert_eq!(s.inputs[3].name, "fork_owner");
        // Prompt from SKILL.md
        match &s.definition.system_prompt {
            PromptSource::Inline(text) => {
                assert!(text.contains("Dev Workflow"), "SKILL.md content present");
                assert!(
                    text.contains("{fork_owner}"),
                    "template placeholders preserved"
                );
            }
            other => panic!("expected inline prompt, got {other:?}"),
        }
    }
}
