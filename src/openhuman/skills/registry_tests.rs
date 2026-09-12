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
    assert!(missing_required_inputs(&defs(), &json!({"repo": "acme/web", "issue": 42})).is_empty());
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

    let get =
        |id: &str, root: Option<&std::path::Path>| get_workflow_with_profile(ws.path(), id, root);

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

/// One workflow lookup must run at most one full on-disk discovery pass.
///
/// `get_workflow_with_profile` used to discover twice when the id was not an
/// exact runnable slug: once inside `load_workflows_with_profile`, then again
/// to resolve a profile display name back to its slug. Discovery re-reads and
/// re-parses every bundle under every root, so the second pass doubled the cost
/// — and doubled every per-parse deprecation warning with it (#6166, #6155).
#[test]
fn workflow_lookup_discovers_once() {
    use super::super::ops_discover::DISCOVERY_CALLS;

    let ws = tempfile::TempDir::new().unwrap();
    let profile_root = tempfile::TempDir::new().unwrap();
    seed_runnable_with_name(
        profile_root.path(),
        "zzcount6166",
        "Counted Assistant",
        "COUNT_BODY",
    );

    // Resolution by display name — the path that took the second pass.
    DISCOVERY_CALLS.with(|c| c.set(0));
    let resolved =
        get_workflow_with_profile(ws.path(), "Counted Assistant", Some(profile_root.path()))
            .expect("display name must still resolve");
    assert_eq!(resolved.definition.id, "zzcount6166");
    assert_eq!(
        DISCOVERY_CALLS.with(|c| c.get()),
        1,
        "display-name lookup must discover the skills tree once, not twice"
    );

    // A miss walks the same fallback and must not discover twice either.
    DISCOVERY_CALLS.with(|c| c.set(0));
    assert!(
        get_workflow_with_profile(ws.path(), "zznosuch6166", Some(profile_root.path())).is_none()
    );
    assert_eq!(
        DISCOVERY_CALLS.with(|c| c.get()),
        1,
        "an unresolvable id must discover the skills tree once, not twice"
    );
}
