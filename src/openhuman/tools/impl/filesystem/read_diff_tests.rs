use super::*;
use serde_json::json;
use tempfile::TempDir;

fn make_tool(dir: &TempDir) -> ReadDiffTool {
    ReadDiffTool::new(dir.path().to_path_buf())
}

#[test]
fn name_is_correct() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(make_tool(&tmp).name(), "read_diff");
}

#[test]
fn description_is_non_empty() {
    let tmp = TempDir::new().unwrap();
    assert!(!make_tool(&tmp).description().is_empty());
}

#[test]
fn schema_is_object_type() {
    let tmp = TempDir::new().unwrap();
    let schema = make_tool(&tmp).parameters_schema();
    assert_eq!(schema["type"], "object");
}

#[test]
fn permission_level_is_read_only() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(
        make_tool(&tmp).permission_level(),
        PermissionLevel::ReadOnly
    );
}

#[tokio::test]
async fn execute_returns_error_for_non_git_dir() {
    let tmp = TempDir::new().unwrap();
    let result = make_tool(&tmp).execute(json!({})).await.unwrap();
    // Non-git dir: git will fail, tool returns error
    assert!(result.is_error);
}

#[tokio::test]
async fn execute_no_changes_in_clean_git_repo() {
    let tmp = TempDir::new().unwrap();
    // Init a git repo and make an initial commit so there's nothing to diff
    let _ = std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(tmp.path())
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "t@t.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "t@t.com")
        .output();
    let result = make_tool(&tmp).execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("No changes found."));
}
