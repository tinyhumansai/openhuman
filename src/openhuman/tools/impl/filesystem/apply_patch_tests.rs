use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        ..SecurityPolicy::default()
    })
}

#[test]
fn apply_patch_name() {
    let tool = ApplyPatchTool::new(test_security(std::env::temp_dir()));
    assert_eq!(tool.name(), "apply_patch");
}

#[tokio::test]
async fn apply_patch_applies_multiple_edits() {
    let dir = std::env::temp_dir().join("openhuman_test_patch_multi");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("a.txt"), "alpha\nbravo")
        .await
        .unwrap();
    tokio::fs::write(dir.join("b.txt"), "one two")
        .await
        .unwrap();

    let tool = ApplyPatchTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({
            "edits": [
                { "path": "a.txt", "old_string": "alpha", "new_string": "ALPHA" },
                { "path": "b.txt", "old_string": "two", "new_string": "TWO" }
            ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let a = tokio::fs::read_to_string(dir.join("a.txt")).await.unwrap();
    let b = tokio::fs::read_to_string(dir.join("b.txt")).await.unwrap();
    assert_eq!(a, "ALPHA\nbravo");
    assert_eq!(b, "one TWO");

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn apply_patch_atomic_on_validation_failure() {
    let dir = std::env::temp_dir().join("openhuman_test_patch_atomic");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("a.txt"), "alpha").await.unwrap();
    tokio::fs::write(dir.join("b.txt"), "bravo").await.unwrap();

    let tool = ApplyPatchTool::new(test_security(dir.clone()));
    // Second edit will fail (no match) — first must NOT be applied.
    let result = tool
        .execute(json!({
            "edits": [
                { "path": "a.txt", "old_string": "alpha", "new_string": "ALPHA" },
                { "path": "b.txt", "old_string": "missing", "new_string": "x" }
            ]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    let a = tokio::fs::read_to_string(dir.join("a.txt")).await.unwrap();
    assert_eq!(a, "alpha", "atomic: first edit must not be persisted");

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn apply_patch_chained_edits_same_file() {
    let dir = std::env::temp_dir().join("openhuman_test_patch_chain");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("a.txt"), "one two three")
        .await
        .unwrap();

    let tool = ApplyPatchTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({
            "edits": [
                { "path": "a.txt", "old_string": "one", "new_string": "ONE" },
                { "path": "a.txt", "old_string": "two", "new_string": "TWO" }
            ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let updated = tokio::fs::read_to_string(dir.join("a.txt")).await.unwrap();
    assert_eq!(updated, "ONE TWO three");

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn apply_patch_rejects_empty_edits() {
    let dir = std::env::temp_dir().join("openhuman_test_patch_empty");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = ApplyPatchTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"edits": []})).await.unwrap();
    assert!(result.is_error);

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn apply_patch_rejects_traversal() {
    let tool = ApplyPatchTool::new(test_security(std::env::temp_dir()));
    let result = tool
        .execute(json!({
            "edits": [
                { "path": "../etc/passwd", "old_string": "x", "new_string": "y" }
            ]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not allowed"));
}
