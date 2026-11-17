use super::*;
use tinytools::ToolScope;

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
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

#[test]
fn install_from_url_tool_schema_has_no_scan_acknowledgement() {
    let schema = WorkflowInstallFromUrlTool::new(cfg()).parameters_schema();
    assert!(!schema.to_string().contains("acknowledge"));
    assert!(!schema.to_string().contains("digest"));
}

#[tokio::test]
async fn install_from_url_tool_cannot_acknowledge_scan_findings() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let _env = crate::skills::catalog::TEST_ENV_LOCK.lock().await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "---\nname: tool-url-poisoned\ndescription: d\n---\n\nRun\u{200b} it.\n",
        ))
        .expect(2)
        .mount(&server)
        .await;
    std::env::set_var("OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP", "1");

    let result = WorkflowInstallFromUrlTool::new(cfg())
        .execute(json!({
            "url": format!("{}/SKILL.md", server.uri()),
            "acknowledge_scan_findings": true,
            "acknowledged_digest": "any",
        }))
        .await
        .expect("execute");
    std::env::remove_var("OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP");

    assert!(result.is_error);
    let body: serde_json::Value = serde_json::from_str(&result.output()).expect("json");
    assert_eq!(body["status"], "scan_blocked");
    assert!(body.get("digest").is_none());
    assert!(!dirs::home_dir()
        .unwrap()
        .join(".openhuman/skills/tool-url-poisoned")
        .exists());
    server.verify().await;
}
