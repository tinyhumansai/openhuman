use super::*;
use crate::openhuman::tools::traits::ToolScope;

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn skill_allowed_respects_optional_allowlist() {
    // None = all skills visible.
    assert!(skill_allowed(&None, "deep-research"));
    // Some(set) restricts to named dir_name slugs.
    let set: std::collections::HashSet<String> =
        ["deep-research".to_string()].into_iter().collect();
    assert!(skill_allowed(&Some(set.clone()), "deep-research"));
    assert!(!skill_allowed(&Some(set), "ship-and-babysit"));
    // Empty allowlist blocks everything (profile selected no skills).
    assert!(!skill_allowed(
        &Some(std::collections::HashSet::new()),
        "anything"
    ));
}

#[tokio::test]
async fn describe_workflow_blocks_disallowed_skill_before_lookup() {
    let allow: std::collections::HashSet<String> =
        ["allowed-skill".to_string()].into_iter().collect();
    let tool = WorkflowDescribeTool::new(cfg()).with_skill_allowlist(Some(allow));
    let res = tool
        .execute(json!({ "workflow_id": "blocked-skill" }))
        .await
        .expect("execute");
    assert!(res.is_error, "disallowed skill must return an error result");
    let text = serde_json::to_string(&res.content).expect("serialize content");
    assert!(
        text.contains("not available to the active agent profile"),
        "expected profile-allowlist rejection, got: {text}"
    );
}

#[tokio::test]
async fn run_history_is_scoped_to_profile_and_allowlist() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    let config = Arc::new(config);
    let run_id = "aaaaaaaa-1111-2222-3333-444444444444";
    let path = crate::openhuman::skills::run_log::run_log_path(tmp.path(), "private-flow", run_id);
    crate::openhuman::skills::run_log::write_header_with_profile(
        &path,
        "private-flow",
        run_id,
        &json!({"secret": true}),
        "private prompt",
        Some("alice"),
    )
    .await
    .expect("write header");

    let mut alice = crate::openhuman::agent::profiles::built_in_profiles()
        .into_iter()
        .next()
        .expect("built-in profile");
    alice.id = "alice".to_string();
    let mut bob = alice.clone();
    bob.id = "bob".to_string();

    let alice_list = WorkflowRecentRunsTool::new(config.clone())
        .with_active_profile(Some(alice.clone()))
        .execute(json!({}))
        .await
        .expect("alice list");
    assert!(alice_list.output_for_llm(false).contains(run_id));

    let bob_list = WorkflowRecentRunsTool::new(config.clone())
        .with_active_profile(Some(bob.clone()))
        .execute(json!({}))
        .await
        .expect("bob list");
    assert!(!bob_list.output_for_llm(false).contains(run_id));

    let bob_read = WorkflowReadRunLogTool::new(config.clone())
        .with_active_profile(Some(bob))
        .execute(json!({"run_id": run_id}))
        .await;
    assert!(bob_read.is_err(), "another profile must not read the log");

    let alice_disallowed = WorkflowReadRunLogTool::new(config)
        .with_active_profile(Some(alice))
        .with_skill_allowlist(Some(std::collections::HashSet::new()))
        .execute(json!({"run_id": run_id}))
        .await;
    assert!(
        alice_disallowed.is_err(),
        "the profile allowlist must also gate run logs"
    );
}

#[test]
fn names_and_levels() {
    let c = cfg();
    assert_eq!(WorkflowListTool::new(c.clone()).name(), "list_workflows");
    assert_eq!(
        WorkflowListTool::new(c.clone()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        WorkflowCreateTool::new(c.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(
        WorkflowInstallFromUrlTool::new(c.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert!(WorkflowInstallFromUrlTool::new(c.clone())
        .external_effect_with_args(&serde_json::Value::Null));
    assert_eq!(
        WorkflowUninstallTool.permission_level(),
        PermissionLevel::Dangerous
    );
    assert_eq!(WorkflowListTool::new(c).scope(), ToolScope::All);
}

#[tokio::test]
async fn describe_requires_workflow_id() {
    let err = WorkflowDescribeTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing workflow_id");
    assert!(err.to_string().contains("workflow_id"));
}

#[tokio::test]
async fn describe_accepts_legacy_skill_id_alias() {
    // `skill_id` still resolves (back-compat) — a non-existent id should
    // fail with "not found", not "missing argument".
    let err = WorkflowDescribeTool::new(cfg())
        .execute(json!({ "skill_id": "does-not-exist" }))
        .await
        .expect_err("unknown workflow");
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn read_resource_requires_both_args() {
    let err = WorkflowReadResourceTool::new(cfg())
        .execute(json!({ "workflow_id": "x" }))
        .await
        .expect_err("missing relative_path");
    assert!(err.to_string().contains("relative_path"));
}

#[tokio::test]
async fn uninstall_requires_name() {
    let err = WorkflowUninstallTool
        .execute(json!({}))
        .await
        .expect_err("missing name");
    assert!(err.to_string().contains("name"));
}

#[tokio::test]
async fn list_returns_envelope() {
    // A fresh workspace has no project workflows, but the user-home scan
    // may surface bundled ones; either way the call succeeds and returns
    // the envelope shape.
    let out = WorkflowListTool::new(cfg())
        .execute(json!({}))
        .await
        .expect("list");
    assert!(out.output_for_llm(false).contains("workflows"));
}
