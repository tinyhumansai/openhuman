use super::*;

// -- create_workflow --------------------------------------------------------

#[test]
fn create_skill_user_scope_scaffolds_skill_md_and_resource_dirs() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateWorkflowParams {
        name: "My Demo Workflow".to_string(),
        description: "Send a friendly greeting to the user.".to_string(),
        when_to_use: None,
        scope: WorkflowScope::User,
        license: Some("MIT".to_string()),
        author: Some("Jane Dev".to_string()),
        tags: vec!["demo".to_string(), "greeting".to_string()],
        allowed_tools: vec!["shell".to_string()],
        inputs: Vec::new(),
        overwrite: false,
    };

    let created = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect("create_workflow should succeed");

    assert_eq!(created.name, "my-demo-workflow");
    assert_eq!(created.scope, WorkflowScope::User);
    assert_eq!(created.description, "Send a friendly greeting to the user.");
    assert_eq!(created.author.as_deref(), Some("Jane Dev"));
    assert_eq!(
        created.tags,
        vec!["demo".to_string(), "greeting".to_string()]
    );
    assert_eq!(created.tools, vec!["shell".to_string()]);

    let skill_root = home
        .path()
        .join(".openhuman")
        .join("workflows")
        .join("my-demo-workflow");
    assert!(skill_root.join(WORKFLOW_MD).is_file());
    for sub in RESOURCE_DIRS {
        assert!(skill_root.join(sub).is_dir(), "missing scaffold dir: {sub}");
    }

    // Frontmatter round-trips through the parser.
    let on_disk = std::fs::read_to_string(skill_root.join(WORKFLOW_MD)).unwrap();
    assert!(on_disk.contains("name: my-demo-workflow"));
    assert!(on_disk.contains("license: MIT"));
    assert!(on_disk.contains("author: Jane Dev"));
}

#[test]
fn create_skill_rejects_slug_collision() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateWorkflowParams {
        name: "collider".to_string(),
        description: "first".to_string(),
        when_to_use: None,
        scope: WorkflowScope::User,
        ..Default::default()
    };
    create_workflow_inner(Some(home.path()), ws.path(), params.clone()).unwrap();

    let err = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect_err("second create with same name must fail");
    assert!(
        err.to_lowercase().contains("already exists"),
        "unexpected error: {err}"
    );
}

#[test]
fn edit_updates_workflow_that_still_lives_under_legacy_skills_root() {
    // Regression: a workflow created before the skills→workflows rename lives
    // at `~/.openhuman/skills/<slug>/SKILL.md`. Editing it (overwrite=true)
    // must resolve that legacy location and update it in place — NOT fail with
    // "cannot update workflow '<slug>': it does not exist at
    // ~/.openhuman/workflows/<slug>" (which only checked the new root).
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let legacy_dir = home
        .path()
        .join(".openhuman")
        .join("skills")
        .join("slack-to-notion");
    write(
        &legacy_dir.join(SKILL_MD),
        "---\nname: slack-to-notion\ndescription: Old description.\n---\n\nOriginal procedure body.\n",
    );

    let params = CreateWorkflowParams {
        name: "slack-to-notion".to_string(),
        description: "Updated description.".to_string(),
        when_to_use: None,
        scope: WorkflowScope::User,
        overwrite: true,
        ..Default::default()
    };
    let updated = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect("editing a legacy-located workflow should succeed");

    assert_eq!(updated.name, "slack-to-notion");
    assert_eq!(updated.scope, WorkflowScope::User);
    assert_eq!(updated.description, "Updated description.");

    // Updated in place under the legacy dir, migrated to WORKFLOW.md...
    let workflow_md = legacy_dir.join(WORKFLOW_MD);
    assert!(
        workflow_md.is_file(),
        "WORKFLOW.md must be written into the legacy skills/ dir"
    );
    // ...with the stale SKILL.md retired so discovery sees no duplicate...
    assert!(
        !legacy_dir.join(SKILL_MD).exists(),
        "legacy SKILL.md must be removed after the in-place migration"
    );
    // ...and the hand-authored body preserved across the edit.
    let body = std::fs::read_to_string(&workflow_md).unwrap();
    assert!(
        body.contains("Original procedure body."),
        "edit must preserve the body; got:\n{body}"
    );
    assert!(
        body.contains("description: Updated description."),
        "edit must rewrite the frontmatter description; got:\n{body}"
    );
    // Nothing should have been created under the new workflows/ root.
    assert!(
        !home
            .path()
            .join(".openhuman")
            .join("workflows")
            .join("slack-to-notion")
            .exists(),
        "in-place edit must not fork a second copy under workflows/"
    );
}

#[test]
fn create_skill_writes_distinct_when_to_use_to_skill_toml_without_inputs() {
    // The unified create form merges the old workflow's `when_to_use` trigger
    // into the skill form. A workflow with a distinct trigger but NO inputs
    // must still get a skill.toml so the trigger persists (and is not just
    // copied from the description).
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateWorkflowParams {
        name: "Triggered Workflow".to_string(),
        description: "Summarise the inbox.".to_string(),
        when_to_use: Some("when the user asks to triage email".to_string()),
        scope: WorkflowScope::User,
        ..Default::default()
    };
    let created = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect("create_workflow should succeed");

    let workflow_md = created.location.expect("created workflow has a location");
    let workflow_toml = workflow_md
        .parent()
        .expect("WORKFLOW.md has a parent dir")
        .join(WORKFLOW_TOML);
    assert!(
        workflow_toml.exists(),
        "workflow.toml must be written when when_to_use is provided, even with no inputs"
    );
    let toml = std::fs::read_to_string(&workflow_toml).unwrap();
    assert!(
        toml.contains("when_to_use = \"when the user asks to triage email\""),
        "skill.toml must carry the distinct trigger, not the description; got:\n{toml}"
    );
    assert!(
        !toml.contains("Summarise the inbox."),
        "when_to_use must NOT fall back to the description when a trigger is given"
    );
}

#[test]
fn create_skill_rejects_non_alphanumeric_name() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateWorkflowParams {
        name: "   ///   ".to_string(),
        description: "nothing useful".to_string(),
        when_to_use: None,
        scope: WorkflowScope::User,
        ..Default::default()
    };
    let err = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect_err("non-alphanumeric name must be rejected");
    // Either the empty-name guard or the slugify guard catches this.
    assert!(
        err.to_lowercase().contains("alphanumeric") || err.to_lowercase().contains("empty"),
        "unexpected error: {err}"
    );
}

#[test]
fn create_skill_rejects_project_scope_without_trust_marker() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    // Intentionally no trust marker.

    let params = CreateWorkflowParams {
        name: "project-skill".to_string(),
        description: "scoped to ws".to_string(),
        when_to_use: None,
        scope: WorkflowScope::Project,
        ..Default::default()
    };
    let err = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect_err("untrusted workspace must reject project scope");
    assert!(
        err.to_lowercase().contains("trust"),
        "unexpected error: {err}"
    );

    // Confirm nothing was written.
    assert!(!ws
        .path()
        .join(".openhuman")
        .join("skills")
        .join("project-skill")
        .exists());
}

#[test]
fn create_skill_project_scope_writes_under_workspace_when_trusted() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    write(&ws.path().join(".openhuman").join(TRUST_MARKER), "");

    let params = CreateWorkflowParams {
        name: "ws-skill".to_string(),
        description: "project-scoped".to_string(),
        when_to_use: None,
        scope: WorkflowScope::Project,
        ..Default::default()
    };
    let created = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect("trusted project-scope create should succeed");

    assert_eq!(created.name, "ws-skill");
    assert_eq!(created.scope, WorkflowScope::Project);
    assert!(ws
        .path()
        .join(".openhuman")
        .join("workflows")
        .join("ws-skill")
        .join(WORKFLOW_MD)
        .is_file());
}

#[test]
fn create_skill_rejects_legacy_scope() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateWorkflowParams {
        name: "legacy-skill".to_string(),
        description: "no".to_string(),
        when_to_use: None,
        scope: WorkflowScope::Legacy,
        ..Default::default()
    };
    let err = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect_err("legacy scope must be rejected");
    assert!(
        err.to_lowercase().contains("legacy"),
        "unexpected error: {err}"
    );
}

#[test]
fn create_skill_rejects_empty_description() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateWorkflowParams {
        name: "ok-name".to_string(),
        description: "   ".to_string(),
        when_to_use: None,
        scope: WorkflowScope::User,
        ..Default::default()
    };
    let err = create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect_err("empty description must be rejected");
    assert!(
        err.to_lowercase().contains("description"),
        "unexpected error: {err}"
    );
}

// -- install URL host policy (portable guards are tested in tinyskills) ------

#[test]
fn normalize_install_url_accepts_a_file_api_that_names_the_md_in_its_query() {
    let url = "https://clawhub.ai/api/v1/skills/apple-design/file?path=SKILL.md";
    assert_eq!(normalize_install_url(url).unwrap(), url);
    let err =
        normalize_install_url("https://clawhub.ai/api/v1/skills/x/file?path=run.sh").unwrap_err();
    assert!(err.contains(".md"), "{err}");
    // Only ClawHub's file endpoint may name the file in its query.
    for other in [
        "https://example.com/api/v1/skills/x/file?path=SKILL.md",
        "https://clawhub.ai/download?path=SKILL.md",
    ] {
        let err = normalize_install_url(other).unwrap_err();
        assert!(err.contains(".md"), "{other}: {err}");
    }
}

#[tokio::test]
async fn install_workflow_from_url_is_idempotent_when_skill_already_exists() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "---\nname: apple-notes\ndescription: Apple Notes access\n---\n\n# Apple Notes\n",
        ))
        .mount(&server)
        .await;

    let workspace = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let params = InstallWorkflowFromUrlParams {
        url: format!("{}/SKILL.md", server.uri()),
        timeout_secs: Some(5),
    };

    // Pass the local-HTTP escape hatch as an explicit param — the env-var
    // path is process-global and races other env-touching tests under
    // parallel execution (#4567).
    let first = install_workflow_from_url_with_home(
        workspace.path(),
        params.clone(),
        Some(home.path()),
        true,
        crate::skills::ops_install::ScanAcknowledgement::Absent,
    )
    .await
    .unwrap()
    .installed()
    .expect("a clean document installs");
    assert_eq!(first.new_skills, vec!["apple-notes"]);

    let second = install_workflow_from_url_with_home(
        workspace.path(),
        params,
        Some(home.path()),
        true,
        crate::skills::ops_install::ScanAcknowledgement::Absent,
    )
    .await
    .unwrap()
    .installed()
    .expect("a repeat install succeeds");
    assert!(second.new_skills.is_empty(), "{second:?}");
    assert!(second.stdout.contains("already installed"), "{second:?}");
}

#[test]
fn install_fetch_status_reporting_suppresses_client_errors_only() {
    assert!(!should_report_install_fetch_status(200));
    assert!(!should_report_install_fetch_status(404));
    assert!(!should_report_install_fetch_status(410));
    assert!(should_report_install_fetch_status(500));
    assert!(should_report_install_fetch_status(502));
}

/// Happy path: install a SKILL.md under a synthetic user home, verify
/// discovery sees it, uninstall, verify discovery no longer sees it and
/// the on-disk dir is gone.
#[test]
fn uninstall_skill_removes_user_scope_dir() {
    let home = tempfile::tempdir().unwrap();
    let skill_dir = home
        .path()
        .join(".openhuman")
        .join("skills")
        .join("weather-helper");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: weather-helper\ndescription: forecasts\n---\n\nbody\n",
    );
    let before = discover_workflows(Some(home.path()), None, false);
    assert_eq!(before.len(), 1, "setup: skill should be discoverable");

    let outcome = uninstall_workflow(
        UninstallWorkflowParams {
            name: "weather-helper".into(),
        },
        Some(home.path()),
    )
    .unwrap();
    assert_eq!(outcome.name, "weather-helper");
    assert_eq!(outcome.scope, WorkflowScope::User);
    assert!(!skill_dir.exists(), "uninstall should remove the dir");

    let after = discover_workflows(Some(home.path()), None, false);
    assert!(after.is_empty(), "discovery should no longer see it");
}
