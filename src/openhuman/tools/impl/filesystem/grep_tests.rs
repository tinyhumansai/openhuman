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
fn grep_name_and_schema() {
    let tool = GrepTool::new(test_security(std::env::temp_dir()));
    assert_eq!(tool.name(), "grep");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["pattern"].is_object());
    assert!(schema["required"]
        .as_array()
        .unwrap()
        .contains(&json!("pattern")));
}

#[tokio::test]
async fn grep_finds_matches() {
    let dir = std::env::temp_dir().join("openhuman_test_grep_finds");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("a.txt"), "alpha\nbravo\ncharlie")
        .await
        .unwrap();
    tokio::fs::write(dir.join("b.txt"), "alpha2").await.unwrap();

    let tool = GrepTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"pattern": "^alpha"})).await.unwrap();
    assert!(!result.is_error);
    let output = result.output();
    assert!(output.contains("a.txt:1:alpha"));
    assert!(output.contains("b.txt:1:alpha2"));
    assert!(!output.contains("bravo"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn grep_invalid_regex() {
    let dir = std::env::temp_dir().join("openhuman_test_grep_invalid");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = GrepTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"pattern": "([unclosed"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Invalid regex"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn grep_case_insensitive() {
    let dir = std::env::temp_dir().join("openhuman_test_grep_ci");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("c.txt"), "Hello World")
        .await
        .unwrap();

    let tool = GrepTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"pattern": "hello", "case_insensitive": true}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("c.txt:1:Hello World"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn grep_blocks_path_traversal() {
    let tool = GrepTool::new(test_security(std::env::temp_dir()));
    let result = tool
        .execute(json!({"pattern": ".", "path": "../.."}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not allowed"));
}

#[tokio::test]
async fn grep_skips_node_modules_and_git() {
    let dir = std::env::temp_dir().join("openhuman_test_grep_skip");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(dir.join("node_modules"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(dir.join(".git")).await.unwrap();
    tokio::fs::write(dir.join("node_modules/x.txt"), "needle")
        .await
        .unwrap();
    tokio::fs::write(dir.join(".git/x.txt"), "needle")
        .await
        .unwrap();
    tokio::fs::write(dir.join("real.txt"), "needle")
        .await
        .unwrap();

    let tool = GrepTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"pattern": "needle"})).await.unwrap();
    assert!(!result.is_error);
    let output = result.output();
    assert!(output.contains("real.txt"));
    assert!(!output.contains("node_modules"));
    assert!(!output.contains(".git"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn grep_respects_max_matches() {
    let dir = std::env::temp_dir().join("openhuman_test_grep_max");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let mut text = String::new();
    for _ in 0..50 {
        text.push_str("hit\n");
    }
    tokio::fs::write(dir.join("many.txt"), text).await.unwrap();

    let tool = GrepTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"pattern": "hit", "max_matches": 5}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("truncated"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

/// A file over `MAX_FILE_BYTES` is silently excluded rather than read — this
/// pins that the exclusion is at least honest about it: the file does not
/// count toward `scanned`, so the report never claims to have searched bytes
/// it skipped. It does not surface the file was too large; a caller who
/// expected a match in it sees only a lower `scanned` count with no reason.
#[tokio::test]
async fn grep_excludes_an_oversized_file_and_does_not_count_it_scanned() {
    let dir = std::env::temp_dir().join("openhuman_test_grep_oversized");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let small = format!("{}\nneedle\n", "line ".repeat(10));
    tokio::fs::write(dir.join("small.txt"), &small)
        .await
        .unwrap();
    let huge = "needle\n".repeat(1_000_000);
    tokio::fs::write(dir.join("huge.txt"), &huge).await.unwrap();

    let tool = GrepTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"pattern": "needle"})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    assert!(
        result.output().contains("small.txt"),
        "the small file's match must still be reported: {}",
        result.output()
    );
    assert!(
        !result.output().contains("huge.txt"),
        "the oversized file must not be scanned at all: {}",
        result.output()
    );
    assert!(
        result.output().contains("scanned 1 file"),
        "the scanned count must not include the file it skipped for size: {}",
        result.output()
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}
