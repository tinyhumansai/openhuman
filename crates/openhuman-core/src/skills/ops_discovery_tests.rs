use super::*;

#[test]
fn init_skills_dir_creates_dir_and_readme() {
    let dir = tempfile::tempdir().unwrap();
    init_workflows_dir(dir.path()).unwrap();
    let skills_dir = dir.path().join("skills");
    assert!(skills_dir.is_dir());
    let readme = skills_dir.join("README.md");
    assert!(readme.exists());
}

#[test]
fn project_skills_skipped_when_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    // No trust marker.
    let skill_dir = ws.join(".openhuman").join("skills").join("unsafe");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: unsafe\ndescription: should not load\n---\n",
    );
    let skills = load_skills_ws(ws);
    assert!(skills.is_empty(), "got {skills:?}");
}

#[test]
fn project_scope_shadows_user_scope_on_collision() {
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();
    write(&ws_dir.path().join(".openhuman").join("trust"), "");

    let user_skill = user_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("greet");
    write(
        &user_skill.join("SKILL.md"),
        "---\nname: greet\ndescription: USER COPY\n---\n",
    );

    let proj_skill = ws_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("greet");
    write(
        &proj_skill.join("SKILL.md"),
        "---\nname: greet\ndescription: PROJECT COPY\n---\n",
    );

    let skills = discover_workflows(Some(user_dir.path()), Some(ws_dir.path()), true);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].description, "PROJECT COPY");
    assert!(skills[0].warnings.iter().any(|w| w.contains("shadowed")));
}

#[test]
fn load_skills_surfaces_user_scope() {
    // load_workflow_metadata now delegates to discover_workflows with dirs::home_dir(),
    // so user-scope skills reach production callers that still hit the
    // backwards-compat shim. Simulate this with an explicit tempdir home
    // via discover_workflows — we can't safely override the process HOME in
    // unit tests.
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();

    let user_skill = user_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("user-only");
    write(
        &user_skill.join("SKILL.md"),
        "---\nname: user-only\ndescription: from user home\n---\n",
    );

    let skills = discover_workflows(
        Some(user_dir.path()),
        Some(ws_dir.path()),
        is_workspace_trusted(ws_dir.path()),
    );
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "user-only");
    assert_eq!(skills[0].scope, WorkflowScope::User);
}

#[test]
fn read_skill_resource_happy_path() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    write(
        &skill_dir.join("scripts").join("hello.sh"),
        "#!/bin/sh\necho hi\n",
    );

    let got = read_workflow_resource(ws, "demo", Path::new("scripts/hello.sh"))
        .expect("read should succeed");
    assert_eq!(got, "#!/bin/sh\necho hi\n");
}

#[test]
fn read_skill_resource_uses_directory_id_when_display_name_differs() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = ws.join("skills").join("demo-slug");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: Demo Display\ndescription: test skill\n---\n",
    );
    write(&skill_dir.join("references").join("note.md"), "slug read");

    let got = read_workflow_resource(ws, "demo-slug", Path::new("references/note.md"))
        .expect("directory id should resolve");
    assert_eq!(got, "slug read");
}

#[test]
fn read_skill_resource_rejects_directory_name_collision() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();

    let named_demo = ws.join("skills").join("alpha");
    write(
        &named_demo.join("SKILL.md"),
        "---\nname: demo\ndescription: display-name collision\n---\n",
    );
    write(&named_demo.join("references").join("note.md"), "alpha");

    let slug_demo = ws.join("skills").join("demo");
    write(
        &slug_demo.join("SKILL.md"),
        "---\nname: slug-demo\ndescription: slug collision\n---\n",
    );
    write(&slug_demo.join("references").join("note.md"), "demo");

    let err = read_workflow_resource(ws, "demo", Path::new("references/note.md"))
        .expect_err("ambiguous directory/name collision must be rejected");
    assert!(
        err.to_lowercase().contains("matches both"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_unknown_skill() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();

    let err = read_workflow_resource(ws, "does-not-exist", Path::new("scripts/x.sh"))
        .expect_err("unknown skill must be rejected");
    assert!(
        err.to_lowercase().contains("not found"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_empty_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    make_legacy_skill(ws, "demo");

    let err = read_workflow_resource(ws, "", Path::new("scripts/x.sh"))
        .expect_err("empty skill_id must be rejected");
    assert!(err.to_lowercase().contains("skill_id"), "unexpected: {err}");

    let err = read_workflow_resource(ws, "demo", Path::new(""))
        .expect_err("empty relative_path must be rejected");
    assert!(err.contains("non-empty relative path"), "unexpected: {err}");
}

// ── `discovery_home_dir`: the per-agent "no user roots" switch ───────────

use crate::skills::ops_discover::discovery_home_dir;

#[tokio::test]
async fn discovery_home_dir_is_hidden_by_a_context_without_user_skill_roots() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    // Outside any scope the ambient default applies: the operator's home.
    // (Whether a DEFAULT_CONTEXT was built by an earlier test in this process
    // does not matter — every booted or test context keeps user roots on.)
    assert_eq!(discovery_home_dir(), dirs::home_dir());

    let mut config = crate::config::Config::default();
    config.workspace_dir = std::path::PathBuf::from("/tmp/discovery-home-dir-test");
    let parent = CoreContext::for_test(DomainSet::full(), None);
    let hidden = parent.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), Default::default())
            .without_user_skill_roots(),
    );
    let visible = parent.derive_with(ContextOverlay::new(
        config,
        DomainSet::full(),
        Default::default(),
    ));

    let under_hidden = CoreContext::scope(hidden, async { discovery_home_dir() }).await;
    assert_eq!(under_hidden, None, "user-scope roots must be hidden");

    let under_visible = CoreContext::scope(visible, async { discovery_home_dir() }).await;
    assert_eq!(under_visible, dirs::home_dir());
}

#[tokio::test]
async fn load_workflow_metadata_skips_user_roots_under_a_hidden_context() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    // A user-scope skill under a fake home would normally be surfaced by
    // `load_workflow_metadata`; a derived context without user roots must not
    // see it, while workspace-scope skills are unaffected.
    let ws_dir = tempfile::tempdir().unwrap();
    let ws_skill = ws_dir.path().join("skills").join("ws-only");
    write(
        &ws_skill.join("SKILL.md"),
        "---\nname: ws-only\ndescription: workspace skill\n---\n",
    );

    let mut config = crate::config::Config::default();
    config.workspace_dir = ws_dir.path().to_path_buf();
    let hidden = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(config, DomainSet::full(), Default::default())
            .without_user_skill_roots(),
    );

    let ws = ws_dir.path().to_path_buf();
    let names: Vec<String> = CoreContext::scope(hidden, async move {
        load_workflow_metadata(&ws)
            .into_iter()
            .map(|w| w.name)
            .collect()
    })
    .await;
    assert!(
        names.iter().any(|n| n == "ws-only"),
        "workspace skills survive: {names:?}"
    );
    assert!(
        names.iter().all(|n| n == "ws-only"),
        "no user-scope skill may leak into a hidden-root context: {names:?}"
    );
}

#[test]
fn workflow_metadata_is_reused_until_invalidated() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = workspace.path().join("skills").join("cached-skill");
    write(
        &skill.join("SKILL.md"),
        "---\nname: cached-skill\ndescription: first version\n---\n",
    );
    let first = load_workflow_metadata(workspace.path());
    assert!(first.iter().any(|item| item.name == "cached-skill"));

    std::fs::remove_dir_all(&skill).unwrap();
    assert!(
        load_workflow_metadata(workspace.path())
            .iter()
            .any(|item| item.name == "cached-skill"),
        "a turn reuses the startup snapshot"
    );

    crate::skills::ops_discover::invalidate_workflow_metadata_cache();
    assert!(
        !load_workflow_metadata(workspace.path())
            .iter()
            .any(|item| item.name == "cached-skill"),
        "a skill change refreshes the snapshot"
    );
}
