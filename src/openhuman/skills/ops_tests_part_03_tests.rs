use super::*;

/// The current user-scope layout is `~/.openhuman/workflows/<id>/SKILL.md`
/// (create writes here post skills→workflows rename). Discovery must surface
/// it as a User-scope workflow.
#[test]
fn discover_reads_user_scope_workflows_dir() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home
            .path()
            .join(".openhuman")
            .join("workflows")
            .join("inbox-triage")
            .join("SKILL.md"),
        "---\nname: inbox-triage\ndescription: triage the inbox\n---\n\nbody\n",
    );
    let found = discover_workflows(Some(home.path()), None, false);
    assert_eq!(
        found.len(),
        1,
        "workflow under .openhuman/workflows/ must load"
    );
    assert_eq!(found[0].name, "inbox-triage");
    assert_eq!(found[0].scope, WorkflowScope::User);
    assert!(!found[0].legacy);
}

/// Names containing path separators or traversal sequences are rejected
/// before any filesystem access.
#[test]
fn uninstall_skill_rejects_path_traversal_names() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".openhuman").join("skills")).unwrap();
    for bad in ["../etc", "foo/bar", "foo\\bar", "..", "foo/../bar"] {
        let err = uninstall_workflow(
            UninstallWorkflowParams { name: bad.into() },
            Some(home.path()),
        )
        .unwrap_err();
        assert!(
            err.contains("path separators") || err.contains("is not installed"),
            "name {bad:?} should be rejected before fs access, got: {err}"
        );
    }
}

/// Empty and whitespace-only names return a clear required-field error.
#[test]
fn uninstall_skill_rejects_empty_name() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".openhuman").join("skills")).unwrap();
    for bad in ["", "   ", "\t"] {
        let err = uninstall_workflow(
            UninstallWorkflowParams { name: bad.into() },
            Some(home.path()),
        )
        .unwrap_err();
        assert!(err.contains("name is required"), "{bad:?} => {err}");
    }
}

/// Uninstalling a skill that is not installed surfaces a recognizable
/// error rather than a generic I/O failure.
#[test]
fn uninstall_skill_missing_skill_errors_cleanly() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".openhuman").join("skills")).unwrap();
    let err = uninstall_workflow(
        UninstallWorkflowParams {
            name: "ghost".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(err.contains("not installed"), "got: {err}");
}

/// A directory that does not contain a `SKILL.md` is refused — we only
/// remove things that look like skills we installed, not arbitrary
/// directories the user dropped in.
#[test]
fn uninstall_skill_refuses_dir_without_skill_md() {
    let home = tempfile::tempdir().unwrap();
    let bogus = home.path().join(".openhuman").join("skills").join("bogus");
    std::fs::create_dir_all(&bogus).unwrap();
    std::fs::write(bogus.join("random.txt"), "not a skill").unwrap();
    let err = uninstall_workflow(
        UninstallWorkflowParams {
            name: "bogus".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(err.contains("does not look like a workflow"), "got: {err}");
    assert!(bogus.exists(), "non-skill dir should not be deleted");
}

/// Delete must work for workflows authored post-rename, i.e. under
/// `~/.openhuman/workflows/<id>/WORKFLOW.md` (the regression that left delete
/// looking only in the legacy `skills/` root).
#[test]
fn uninstall_workflow_removes_new_workflows_dir() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".openhuman").join("workflows").join("wf");
    write(
        &dir.join(WORKFLOW_MD),
        "---\nname: wf\ndescription: d\n---\n\nbody\n",
    );
    let out = uninstall_workflow(
        UninstallWorkflowParams { name: "wf".into() },
        Some(home.path()),
    )
    .expect("delete should succeed for a workflows/ dir");
    assert_eq!(out.name, "wf");
    assert!(!dir.exists(), "workflow dir should be removed");
}

/// A symlink inside the skills root pointing outside the root must be
/// rejected by the raw-path symlink preflight before `canonicalize`
/// would follow the link. The earlier `starts_with` / `is_dir` guards
/// remain as defence-in-depth for anything that slips past the
/// preflight on future refactors.
#[cfg(unix)]
#[test]
fn uninstall_skill_rejects_symlink_escape() {
    let home = tempfile::tempdir().unwrap();
    let skills_root = home.path().join(".openhuman").join("skills");
    std::fs::create_dir_all(&skills_root).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("real");
    write(
        &target.join("SKILL.md"),
        "---\nname: real\ndescription: out of tree\n---\n",
    );
    std::os::unix::fs::symlink(&target, skills_root.join("real")).unwrap();
    let err = uninstall_workflow(
        UninstallWorkflowParams {
            name: "real".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(
        err.contains("symlinked alias")
            || err.contains("path escapes skills root")
            || err.contains("is not a directory"),
        "symlink out of tree must be rejected, got: {err}"
    );
    assert!(target.exists(), "symlink target must not be deleted");
}

/// An in-tree symlink alias (`skills/alias -> skills/real`) must be
/// rejected even though it does not escape the skills root — otherwise
/// the uninstall of `alias` would nuke the real skill directory behind
/// it, violating the invariant that the named slug is deleted.
#[cfg(unix)]
#[test]
fn uninstall_skill_rejects_symlinked_alias_in_tree() {
    let home = tempfile::tempdir().unwrap();
    let skills_root = home.path().join(".openhuman").join("skills");
    std::fs::create_dir_all(&skills_root).unwrap();
    let real_dir = skills_root.join("real");
    write(
        &real_dir.join("SKILL.md"),
        "---\nname: real\ndescription: in tree\n---\n",
    );
    std::os::unix::fs::symlink(&real_dir, skills_root.join("alias")).unwrap();
    let err = uninstall_workflow(
        UninstallWorkflowParams {
            name: "alias".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(
        err.contains("symlinked alias"),
        "in-tree alias must be rejected by preflight, got: {err}"
    );
    assert!(
        real_dir.join("SKILL.md").exists(),
        "real skill behind the alias must survive"
    );
}

/// A symlinked skills *root* (`~/.openhuman/skills -> elsewhere`) must
/// be refused before canonicalisation, since `canonicalize` would
/// resolve it to the target and the `starts_with` guard would then
/// compare against the resolved target, not the nominal root.
#[cfg(unix)]
#[test]
fn uninstall_skill_rejects_symlinked_skills_root() {
    let home = tempfile::tempdir().unwrap();
    let real_root = tempfile::tempdir().unwrap();
    let real_skills = real_root.path().join("skills");
    std::fs::create_dir_all(&real_skills).unwrap();
    write(
        &real_skills.join("real").join("SKILL.md"),
        "---\nname: real\ndescription: in real root\n---\n",
    );
    std::fs::create_dir_all(home.path().join(".openhuman")).unwrap();
    std::os::unix::fs::symlink(&real_skills, home.path().join(".openhuman").join("skills"))
        .unwrap();
    let err = uninstall_workflow(
        UninstallWorkflowParams {
            name: "real".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(
        err.contains("symlink"),
        "symlinked workflows root must be refused, got: {err}"
    );
    assert!(
        real_skills.join("real").join("SKILL.md").exists(),
        "target must survive"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// `[[inputs]]` editor — Phase 1: schema round-trip.
//
// The Create-a-Workflow form lets the user declare zero-or-more skill inputs at
// create time. These tests pin the wire shape and the params round-trip so the
// payload from `skillsApi.createSkill` lands intact in `CreateWorkflowParams.inputs`
// and is identical after TOML emission + re-parse via the registry's
// `WorkflowInput` (see Phase 2 for the actual on-disk emit; Phase 1 is JSON only).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn skill_create_input_def_deserializes_full_row_from_json() {
    let row: crate::openhuman::skills::ops_create::WorkflowCreateInputDef =
        serde_json::from_value(serde_json::json!({
            "name": "repo",
            "description": "owner/name slug",
            "required": true,
            "type": "string",
        }))
        .unwrap();
    assert_eq!(row.name, "repo");
    assert_eq!(row.description.as_deref(), Some("owner/name slug"));
    assert!(row.required);
    assert_eq!(row.type_.as_deref(), Some("string"));
}

#[test]
fn skill_create_input_def_required_defaults_to_true() {
    // The form sends `required` per row, but other callers (CLI, future
    // RPC clients) may omit it. The serde default keeps the safer
    // semantic — a row the user bothered to declare is required.
    let row: crate::openhuman::skills::ops_create::WorkflowCreateInputDef =
        serde_json::from_value(serde_json::json!({
            "name": "topic",
        }))
        .unwrap();
    assert_eq!(row.name, "topic");
    assert!(row.description.is_none());
    assert!(row.required, "required must default to true");
    assert!(row.type_.is_none());
}

#[test]
fn create_skill_params_defaults_inputs_to_empty_vec() {
    // Old clients that don't know about `inputs` keep working — the
    // field defaults to an empty vec at deserialise time and `Default`
    // produces an empty vec too.
    let params: CreateWorkflowParams = serde_json::from_value(serde_json::json!({
        "name": "Hello",
        "description": "Says hi",
        "scope": "user",
    }))
    .unwrap();
    assert!(params.inputs.is_empty());
    assert!(CreateWorkflowParams::default().inputs.is_empty());
}

#[test]
fn create_skill_params_carries_inputs_through_deserialise() {
    let params: CreateWorkflowParams = serde_json::from_value(serde_json::json!({
        "name": "Issue Crusher",
        "description": "Fix one issue end to end.",
        "scope": "user",
        "inputs": [
            { "name": "repo", "description": "owner/name", "required": true, "type": "string" },
            { "name": "issue", "description": "issue #", "required": true, "type": "integer" },
            { "name": "pr_base", "description": "base branch", "required": false }
        ],
    }))
    .unwrap();
    assert_eq!(params.inputs.len(), 3);
    assert_eq!(params.inputs[1].name, "issue");
    assert_eq!(params.inputs[1].type_.as_deref(), Some("integer"));
    assert!(params.inputs[1].required);
    assert!(!params.inputs[2].required);
    assert!(params.inputs[2].type_.is_none());
}

#[test]
fn skill_create_input_def_round_trips_through_registry_skill_input() {
    // Asserts that what the form emits and what the registry parser
    // accepts are the same shape over TOML — the "parser will accept
    // what you emit" contract called out in the Phase-1 brief. We
    // serialise the form-supplied row(s) into a synthetic skill.toml
    // body, parse it back through the registry's `WorkflowDefinition`,
    // and check every field survived.
    let rows = vec![
        crate::openhuman::skills::ops_create::WorkflowCreateInputDef {
            name: "repo".into(),
            description: Some("owner/name slug".into()),
            required: true,
            type_: Some("string".into()),
        },
        crate::openhuman::skills::ops_create::WorkflowCreateInputDef {
            name: "issue".into(),
            description: Some("issue #".into()),
            required: true,
            type_: Some("integer".into()),
        },
        crate::openhuman::skills::ops_create::WorkflowCreateInputDef {
            name: "pr_base".into(),
            description: None,
            required: false,
            type_: None,
        },
    ];

    // Hand-build a minimal skill.toml the registry can parse: id +
    // when_to_use are the only AgentDefinition fields without defaults.
    let mut toml = String::from("id = \"round-trip\"\nwhen_to_use = \"trip\"\n");
    for r in &rows {
        toml.push_str("\n[[inputs]]\n");
        toml.push_str(&format!("name = \"{}\"\n", r.name));
        if let Some(d) = &r.description {
            toml.push_str(&format!("description = \"{}\"\n", d));
        }
        toml.push_str(&format!("required = {}\n", r.required));
        if let Some(t) = &r.type_ {
            toml.push_str(&format!("type = \"{}\"\n", t));
        }
    }

    let parsed: crate::openhuman::skills::registry::WorkflowDefinition =
        toml::from_str(&toml).expect("registry must accept what the form emits");
    assert_eq!(parsed.inputs.len(), 3);
    assert_eq!(parsed.inputs[0].name, "repo");
    assert_eq!(parsed.inputs[0].description, "owner/name slug");
    assert!(parsed.inputs[0].required);
    assert_eq!(parsed.inputs[0].kind.as_deref(), Some("string"));
    assert_eq!(parsed.inputs[1].kind.as_deref(), Some("integer"));
    // `description` defaults to "" in `WorkflowInput`, not Option::None —
    // the registry parser flattens missing into empty for back-compat.
    assert_eq!(parsed.inputs[2].description, "");
    assert!(!parsed.inputs[2].required);
    assert!(parsed.inputs[2].kind.is_none());
}

#[test]
fn edit_refuses_to_overwrite_an_unparseable_body() {
    // Data-loss guard: when editing a workflow whose existing markdown can't be
    // parsed (here an unterminated frontmatter block), the update must fail
    // rather than silently replace the user's instructions with the scaffold.
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".openhuman")
        .join("skills")
        .join("broken-wf");
    // `---` opened but never closed → parse_workflow_md returns None.
    write(
        &dir.join(SKILL_MD),
        "---\nname: broken-wf\ndescription: x\n",
    );

    let params = CreateWorkflowParams {
        name: "broken-wf".to_string(),
        description: "new description".to_string(),
        when_to_use: None,
        scope: WorkflowScope::User,
        overwrite: true,
        ..Default::default()
    };
    let err = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect_err("edit must refuse when the existing body can't be parsed");
    assert!(
        err.to_lowercase().contains("could not be parsed"),
        "unexpected error: {err}"
    );
    // The original file is left untouched and no WORKFLOW.md scaffold is written.
    let still = std::fs::read_to_string(dir.join(SKILL_MD)).unwrap();
    assert!(still.contains("name: broken-wf"), "original must be intact");
    assert!(
        !dir.join(WORKFLOW_MD).exists(),
        "no scaffold WORKFLOW.md should be written on a refused edit"
    );
}

#[test]
fn uninstall_resolves_agents_skills_legacy_root() {
    // discover_workflows surfaces ~/.agents/skills/, so uninstall must reach it
    // too — otherwise a listed workflow can never be deleted via this API.
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".agents").join("skills").join("agenty");
    write(
        &dir.join(SKILL_MD),
        "---\nname: agenty\ndescription: legacy root\n---\n\nbody\n",
    );

    let outcome = uninstall_workflow(
        UninstallWorkflowParams {
            name: "agenty".into(),
        },
        Some(home.path()),
    )
    .expect("uninstall should resolve the ~/.agents/skills/ legacy root");
    assert_eq!(outcome.name, "agenty");
    assert!(!dir.exists(), "uninstall should remove the dir");
}

// Unix-only: exercises `std::os::unix::fs::symlink`. Windows symlink creation
// uses a different API and requires elevated privileges / Developer Mode, so
// this case is gated off there (the Windows lib test binary otherwise fails to
// compile on this line).
#[cfg(unix)]
#[test]
fn symlinked_manifest_file_is_rejected() {
    // `exists()` follows symlinks; a manifest pointed at an external file would
    // otherwise be ingested into the catalog/prompt flow. Discovery must skip it.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join(TRUST_MARKER), "");
    let skill_dir = ws.join(".openhuman").join("skills").join("sneaky");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let external = dir.path().join("secret.md");
    write(
        &external,
        "---\nname: sneaky\ndescription: leaked\n---\n\nsecret\n",
    );
    std::os::unix::fs::symlink(&external, skill_dir.join(WORKFLOW_MD)).unwrap();

    let skills = load_skills_ws(ws);
    assert!(
        skills.iter().all(|s| s.name != "sneaky"),
        "a symlinked manifest must not be loaded; got {skills:?}"
    );
}

// ---------------------------------------------------------------------------
// discover_automations: the Automations UI list shows only `workflows/`-root
// task templates, never capability skills under `skills/` roots. The full
// surface (`discover_workflows`) still includes both for the agent harness.
// ---------------------------------------------------------------------------

#[test]
fn discover_automations_excludes_user_skill_root_but_keeps_workflows() {
    let home = tempfile::tempdir().unwrap();
    // A capability skill installed under ~/.openhuman/skills/.
    write(
        &home
            .path()
            .join(".openhuman")
            .join("skills")
            .join("ascii-art")
            .join(SKILL_MD),
        "---\nname: ascii-art\ndescription: ASCII art\n---\n",
    );
    // A real automation authored under ~/.openhuman/workflows/.
    write(
        &home
            .path()
            .join(".openhuman")
            .join("workflows")
            .join("deploy")
            .join(WORKFLOW_MD),
        "---\nname: deploy\ndescription: Ship a release\n---\n",
    );

    let automations = discover_automations(Some(home.path()), None, false);
    let names: Vec<&str> = automations.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["deploy"],
        "automations must exclude the skills/ root; got {names:?}"
    );

    // The full surface still sees both (agent harness / run paths rely on this).
    let all = discover_workflows(Some(home.path()), None, false);
    let all_names: Vec<&str> = all.iter().map(|w| w.name.as_str()).collect();
    assert!(all_names.contains(&"ascii-art"), "got {all_names:?}");
    assert!(all_names.contains(&"deploy"), "got {all_names:?}");
}

#[test]
fn discover_automations_excludes_agents_skills_root() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home
            .path()
            .join(".agents")
            .join("skills")
            .join("example_skill")
            .join(SKILL_MD),
        "---\nname: example_skill\ndescription: Example skill\n---\n",
    );

    let automations = discover_automations(Some(home.path()), None, false);
    assert!(
        automations.is_empty(),
        ".agents/skills bundles are skills, not automations; got {automations:?}"
    );
    let all = discover_workflows(Some(home.path()), None, false);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "example_skill");
}

#[test]
fn discover_automations_excludes_legacy_workspace_skills_root() {
    let ws = tempfile::tempdir().unwrap();
    write(
        &ws.path().join("skills").join("sketch").join(SKILL_MD),
        "---\nname: sketch\ndescription: HTML mockups\n---\n",
    );

    let automations = discover_automations(None, Some(ws.path()), false);
    assert!(
        automations.is_empty(),
        "legacy <workspace>/skills is a skill root; got {automations:?}"
    );
    // Full surface still scans legacy for back-compat.
    let all = discover_workflows(None, Some(ws.path()), false);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "sketch");
    assert_eq!(all[0].scope, WorkflowScope::Legacy);
}

#[test]
fn discover_automations_includes_project_workflows_when_trusted() {
    let ws = tempfile::tempdir().unwrap();
    write(&ws.path().join(".openhuman").join(TRUST_MARKER), "");
    write(
        &ws.path()
            .join(".openhuman")
            .join("workflows")
            .join("proj-flow")
            .join(WORKFLOW_MD),
        "---\nname: proj-flow\ndescription: Project automation\n---\n",
    );
    // A sibling project skill must NOT leak into the automations list.
    write(
        &ws.path()
            .join(".openhuman")
            .join("skills")
            .join("proj-skill")
            .join(SKILL_MD),
        "---\nname: proj-skill\ndescription: Project skill\n---\n",
    );

    let automations = discover_automations(None, Some(ws.path()), true);
    let names: Vec<&str> = automations.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, vec!["proj-flow"], "got {names:?}");
}
