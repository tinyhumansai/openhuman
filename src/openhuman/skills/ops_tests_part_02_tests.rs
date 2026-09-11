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

#[test]
fn slugify_collapses_separators_and_trims() {
    assert_eq!(
        slugify_workflow_name("Hello  World").unwrap(),
        "hello-world"
    );
    assert_eq!(slugify_workflow_name("--foo__bar--").unwrap(), "foo-bar");
    assert_eq!(
        slugify_workflow_name("ALL CAPS skill!").unwrap(),
        "all-caps-skill"
    );
    assert!(slugify_workflow_name("   ").is_err());
    assert!(slugify_workflow_name("!!!").is_err());
}

#[test]
fn validate_install_url_accepts_public_https() {
    for url in &[
        "https://registry.npmjs.org/@acme/skill",
        "https://example.com/skill.tar.gz",
        "https://github.com/acme/skill/releases/download/v1/skill.tgz",
        "https://8.8.8.8/x",
    ] {
        validate_install_url(url).unwrap_or_else(|e| panic!("{url} rejected: {e}"));
    }
}

#[test]
fn validate_install_url_rejects_non_https_scheme() {
    for url in &[
        "http://example.com/x",
        "ftp://example.com/x",
        "file:///etc/passwd",
        "git+ssh://git@example.com/repo",
        "javascript:alert(1)",
    ] {
        assert!(
            validate_install_url(url).is_err(),
            "{url} should be rejected"
        );
    }
}

#[test]
fn validate_install_url_rejects_empty_and_oversized() {
    assert!(validate_install_url("").is_err());
    assert!(validate_install_url("   ").is_err());
    let huge = format!("https://example.com/{}", "a".repeat(MAX_INSTALL_URL_LEN));
    assert!(validate_install_url(&huge).is_err());
}

#[test]
fn validate_install_url_rejects_private_and_loopback() {
    for url in &[
        "https://localhost/x",
        "https://foo.localhost/x",
        "https://foo.local/x",
        "https://127.0.0.1/x",
        "https://127.42.1.1/x",
        "https://10.0.0.5/x",
        "https://172.16.0.1/x",
        "https://172.31.255.255/x",
        "https://192.168.1.1/x",
        "https://169.254.169.254/x", // cloud metadata IP
        "https://100.64.0.1/x",      // CGN
        "https://0.0.0.0/x",
        "https://255.255.255.255/x",
        "https://224.0.0.1/x", // multicast
        "https://[::1]/x",
        "https://[::]/x",
        "https://[fe80::1]/x",
        "https://[fc00::1]/x",
        "https://[fd12:3456:789a::1]/x",
        "https://[ff02::1]/x",
    ] {
        assert!(
            validate_install_url(url).is_err(),
            "{url} should be rejected"
        );
    }
}

#[test]
fn validate_install_url_rejects_malformed() {
    // missing scheme -> parse error
    assert!(validate_install_url("not-a-url").is_err());
    // special scheme with empty host -> parse error
    assert!(validate_install_url("https://").is_err());
    // non-https scheme rejected even when otherwise well-formed
    assert!(validate_install_url("ftp://example.com/x").is_err());
    // unparseable bracketed host
    assert!(validate_install_url("https://[not-an-ip]/x").is_err());
}

#[test]
fn normalize_install_url_rewrites_github_blob_to_raw() {
    let out =
        normalize_install_url("https://github.com/owner/repo/blob/main/path/to/SKILL.md").unwrap();
    assert_eq!(
        out,
        "https://raw.githubusercontent.com/owner/repo/main/path/to/SKILL.md"
    );
}

#[test]
fn normalize_install_url_rewrites_github_blob_nested_path() {
    let out = normalize_install_url("https://github.com/owner/repo/blob/feat/x/dir/sub/SKILL.md")
        .unwrap();
    assert_eq!(
        out,
        "https://raw.githubusercontent.com/owner/repo/feat/x/dir/sub/SKILL.md"
    );
}

#[test]
fn normalize_install_url_passes_raw_github_through() {
    let raw = "https://raw.githubusercontent.com/owner/repo/main/SKILL.md";
    assert_eq!(normalize_install_url(raw).unwrap(), raw);
}

#[test]
fn normalize_install_url_rejects_tree_urls() {
    let err = normalize_install_url("https://github.com/owner/repo/tree/main/path").unwrap_err();
    assert!(err.contains("unsupported url form"), "{err}");
    assert!(err.contains("tree/dir"), "{err}");
}

#[test]
fn normalize_install_url_rejects_whole_repo() {
    let err = normalize_install_url("https://github.com/owner/repo").unwrap_err();
    assert!(err.contains("unsupported url form"), "{err}");
    assert!(err.contains("whole-repo"), "{err}");
}

#[test]
fn normalize_install_url_rejects_non_md_suffix() {
    let err = normalize_install_url("https://example.com/skill.txt").unwrap_err();
    assert!(err.contains("unsupported url form"), "{err}");
    assert!(err.contains(".md"), "{err}");
}

#[test]
fn normalize_install_url_accepts_uppercase_md_suffix() {
    let raw = "https://example.com/SKILL.MD";
    assert_eq!(normalize_install_url(raw).unwrap(), raw);
}

#[test]
fn derive_install_slug_prefers_metadata_id() {
    let mut fm = WorkflowFrontmatter {
        name: "My Workflow".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    fm.metadata.insert(
        "id".to_string(),
        serde_yaml::Value::String("canonical-id".to_string()),
    );
    assert_eq!(derive_install_slug(&fm).unwrap(), "canonical-id");
}

#[test]
fn derive_install_slug_sanitizes_name_fallback() {
    let fm = WorkflowFrontmatter {
        name: "My Cool Workflow!!".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    assert_eq!(derive_install_slug(&fm).unwrap(), "my-cool-workflow");
}

#[test]
fn derive_install_slug_collapses_runs_and_trims_edges() {
    let fm = WorkflowFrontmatter {
        name: "---foo__bar  baz---".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    assert_eq!(derive_install_slug(&fm).unwrap(), "foo-bar-baz");
}

#[test]
fn derive_install_slug_rejects_empty_after_sanitize() {
    let fm = WorkflowFrontmatter {
        name: "!!!".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    let err = derive_install_slug(&fm).unwrap_err();
    assert!(err.contains("invalid SKILL.md"), "{err}");
}

#[test]
fn derive_install_slug_rejects_oversized() {
    let fm = WorkflowFrontmatter {
        name: "a".repeat(MAX_NAME_LEN + 1),
        description: "x".to_string(),
        ..Default::default()
    };
    let err = derive_install_slug(&fm).unwrap_err();
    assert!(err.contains("invalid SKILL.md"), "{err}");
    assert!(err.contains("exceeds"), "{err}");
}

#[test]
fn derive_install_slug_sanitizes_path_escape_attempts() {
    // `..` and `/` are non-alphanumeric so they collapse to `-` during
    // sanitization — verify no path-escape characters survive.
    let fm = WorkflowFrontmatter {
        name: "../etc/passwd".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    let slug = derive_install_slug(&fm).unwrap();
    assert!(!slug.contains(".."), "slug leaked ..: {slug}");
    assert!(!slug.contains('/'), "slug leaked /: {slug}");
    assert!(!slug.contains('\\'), "slug leaked \\: {slug}");
}

#[test]
fn parse_skill_md_str_happy_path() {
    let content = "---\nname: demo\ndescription: a demo skill\n---\n\n# Body\n";
    let (fm, body, warnings) = parse_workflow_md_str(content).unwrap();
    assert_eq!(fm.name, "demo");
    assert_eq!(fm.description, "a demo skill");
    assert!(body.contains("# Body"));
    assert!(warnings.is_empty());
}

#[test]
fn parse_skill_md_str_unterminated_frontmatter_returns_none() {
    let content = "---\nname: demo\ndescription: missing close\n# Body\n";
    assert!(parse_workflow_md_str(content).is_none());
}

#[test]
fn parse_skill_md_str_no_frontmatter_treats_whole_as_body() {
    let content = "# Just a body\nno frontmatter here\n";
    let (fm, body, warnings) = parse_workflow_md_str(content).unwrap();
    assert!(fm.name.is_empty());
    assert_eq!(body, content);
    assert!(warnings.is_empty());
}

#[test]
fn parse_skill_md_str_bad_yaml_returns_empty_frontmatter_with_warning() {
    let content = "---\nname: [unterminated\ndescription: also bad\n---\n";
    let (fm, _body, warnings) = parse_workflow_md_str(content).unwrap();
    assert!(fm.name.is_empty());
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("frontmatter parse error")),
        "expected warning, got {warnings:?}"
    );
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
    )
    .await
    .unwrap();
    assert_eq!(first.new_skills, vec!["apple-notes"]);

    let second =
        install_workflow_from_url_with_home(workspace.path(), params, Some(home.path()), true)
            .await
            .unwrap();
    assert!(second.new_skills.is_empty(), "{second:?}");
    assert!(second.stdout.contains("already installed"), "{second:?}");
}

#[test]
fn install_fetch_status_reporting_suppresses_client_errors_only() {
    assert!(!should_report_install_fetch_status(reqwest::StatusCode::OK));
    assert!(!should_report_install_fetch_status(
        reqwest::StatusCode::NOT_FOUND
    ));
    assert!(!should_report_install_fetch_status(
        reqwest::StatusCode::GONE
    ));
    assert!(should_report_install_fetch_status(
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    ));
    assert!(should_report_install_fetch_status(
        reqwest::StatusCode::BAD_GATEWAY
    ));
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
