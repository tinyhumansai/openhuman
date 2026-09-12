use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        action_dir: workspace.clone(),
        workspace_dir: workspace,
        ..SecurityPolicy::default()
    })
}

#[test]
fn list_name() {
    let tool = ListFilesTool::new(test_security(std::env::temp_dir()));
    assert_eq!(tool.name(), "list");
}

#[tokio::test]
async fn list_lists_files_and_dirs() {
    let dir = std::env::temp_dir().join("openhuman_test_list");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(dir.join("sub")).await.unwrap();
    tokio::fs::write(dir.join("a.txt"), "x").await.unwrap();

    let tool = ListFilesTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    let output = result.output();
    assert!(output.contains("file\ta.txt"));
    assert!(output.contains("dir\tsub"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn list_blocks_path_traversal() {
    let tool = ListFilesTool::new(test_security(std::env::temp_dir()));
    let result = tool.execute(json!({"path": "../../etc"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not allowed"));
}

#[tokio::test]
async fn list_missing_dir() {
    let dir = std::env::temp_dir().join("openhuman_test_list_missing");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let tool = ListFilesTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"path": "nope"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Failed to resolve"));
    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn list_a_plain_file_reports_the_read_dir_failure() {
    // Distinct from `list_missing_dir`: the path resolves (the file exists),
    // so `validate_path` succeeds; the failure is `read_dir` itself refusing
    // a non-directory (ENOTDIR), one step later in the same function.
    let dir = std::env::temp_dir().join("openhuman_test_list_not_a_dir");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("plain.txt"), "not a directory")
        .await
        .unwrap();

    let tool = ListFilesTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"path": "plain.txt"})).await.unwrap();
    assert!(
        result.is_error,
        "listing a plain file must be a refusal, not an empty-looking success"
    );
    assert!(result.output().contains("Failed to read directory"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}
