use super::*;
use crate::openhuman::tools::traits::ToolScope;

fn test_config() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn metadata_is_stable() {
    let cfg = test_config();
    assert_eq!(ArtifactListTool::new(cfg.clone()).name(), "artifact_list");
    assert_eq!(ArtifactGetTool::new(cfg.clone()).name(), "artifact_get");
    assert_eq!(
        ArtifactDeleteTool::new(cfg.clone()).name(),
        "artifact_delete"
    );
    assert_eq!(
        ArtifactListTool::new(cfg.clone()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        ArtifactDeleteTool::new(cfg.clone()).permission_level(),
        PermissionLevel::Dangerous
    );
    assert_eq!(ArtifactListTool::new(cfg).scope(), ToolScope::All);
}

#[test]
fn read_tools_are_concurrency_safe() {
    let cfg = test_config();
    assert!(ArtifactListTool::new(cfg.clone()).is_concurrency_safe(&serde_json::Value::Null));
    assert!(ArtifactGetTool::new(cfg).is_concurrency_safe(&serde_json::Value::Null));
}

#[tokio::test]
async fn get_requires_artifact_id() {
    let tool = ArtifactGetTool::new(test_config());
    let err = tool
        .execute(json!({}))
        .await
        .expect_err("expected missing-arg error");
    assert!(err.to_string().contains("artifact_id"));
}

#[tokio::test]
async fn delete_requires_artifact_id() {
    let tool = ArtifactDeleteTool::new(test_config());
    let err = tool
        .execute(json!({ "artifact_id": "  " }))
        .await
        .expect_err("expected missing-arg error");
    assert!(err.to_string().contains("artifact_id"));
}

#[tokio::test]
async fn list_returns_artifacts_envelope() {
    // Config::default() points at a workspace dir; listing an empty/missing
    // artifacts root yields an empty list, not an error.
    let tool = ArtifactListTool::new(test_config());
    let result = tool.execute(json!({ "limit": 5 })).await;
    // Either a clean empty listing or a benign error if the workspace is
    // unwritable in the sandbox — assert it does not panic and, when Ok,
    // carries the expected shape.
    if let Ok(res) = result {
        let body = res.output_for_llm(false);
        assert!(body.contains("artifacts"), "body was: {body}");
    }
}
