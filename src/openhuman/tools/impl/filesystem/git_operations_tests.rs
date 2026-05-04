use super::*;
use crate::openhuman::security::SecurityPolicy;
use tempfile::TempDir;

fn test_tool(dir: &std::path::Path) -> GitOperationsTool {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    });
    GitOperationsTool::new(security, dir.to_path_buf())
}

#[test]
fn sanitize_git_blocks_injection() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    // Should block dangerous arguments
    assert!(tool.sanitize_git_args("--exec=rm -rf /").is_err());
    assert!(tool.sanitize_git_args("$(echo pwned)").is_err());
    assert!(tool.sanitize_git_args("`malicious`").is_err());
    assert!(tool.sanitize_git_args("arg | cat").is_err());
    assert!(tool.sanitize_git_args("arg; rm file").is_err());
}

#[test]
fn sanitize_git_blocks_pager_editor_injection() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    assert!(tool.sanitize_git_args("--pager=less").is_err());
    assert!(tool.sanitize_git_args("--editor=vim").is_err());
}

#[test]
fn sanitize_git_blocks_config_injection() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    // Exact `-c` flag (config injection)
    assert!(tool.sanitize_git_args("-c core.sshCommand=evil").is_err());
    assert!(tool.sanitize_git_args("-c=core.pager=less").is_err());
}

#[test]
fn sanitize_git_blocks_no_verify() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    assert!(tool.sanitize_git_args("--no-verify").is_err());
}

#[test]
fn sanitize_git_blocks_redirect_in_args() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    assert!(tool.sanitize_git_args("file.txt > /tmp/out").is_err());
}

#[test]
fn sanitize_git_cached_not_blocked() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    // --cached must NOT be blocked by the `-c` check
    assert!(tool.sanitize_git_args("--cached").is_ok());
    // Other safe flags starting with -c prefix
    assert!(tool.sanitize_git_args("-cached").is_ok());
}

#[test]
fn sanitize_git_allows_safe() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    // Should allow safe arguments
    assert!(tool.sanitize_git_args("main").is_ok());
    assert!(tool.sanitize_git_args("feature/test-branch").is_ok());
    assert!(tool.sanitize_git_args("--cached").is_ok());
    assert!(tool.sanitize_git_args("src/main.rs").is_ok());
    assert!(tool.sanitize_git_args(".").is_ok());
}

#[test]
fn requires_write_detection() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    assert!(tool.requires_write_access("commit"));
    assert!(tool.requires_write_access("add"));
    assert!(tool.requires_write_access("checkout"));

    assert!(!tool.requires_write_access("status"));
    assert!(!tool.requires_write_access("diff"));
    assert!(!tool.requires_write_access("log"));
}

#[test]
fn branch_is_not_write_gated() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    // Branch listing is read-only; it must not require write access
    assert!(!tool.requires_write_access("branch"));
    assert!(tool.is_read_only("branch"));
}

#[test]
fn is_read_only_detection() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    assert!(tool.is_read_only("status"));
    assert!(tool.is_read_only("diff"));
    assert!(tool.is_read_only("log"));
    assert!(tool.is_read_only("branch"));

    assert!(!tool.is_read_only("commit"));
    assert!(!tool.is_read_only("add"));
}

#[tokio::test]
async fn blocks_readonly_mode_for_write_ops() {
    let tmp = TempDir::new().unwrap();
    // Initialize a git repository
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

    let result = tool
        .execute(json!({"operation": "commit", "message": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    // can_act() returns false for ReadOnly, so we get the "higher autonomy level" message
    assert!(result.output().contains("higher autonomy"));
}

#[tokio::test]
async fn allows_branch_listing_in_readonly_mode() {
    let tmp = TempDir::new().unwrap();
    // Initialize a git repository so the command can succeed
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

    let result = tool.execute(json!({"operation": "branch"})).await.unwrap();
    // Branch listing must not be blocked by read-only autonomy
    let error_msg = result.output();
    assert!(
        !error_msg.contains("read-only") && !error_msg.contains("higher autonomy"),
        "branch listing should not be blocked in read-only mode, got: {error_msg}"
    );
}

#[tokio::test]
async fn allows_readonly_ops_in_readonly_mode() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

    // This will fail because there's no git repo, but it shouldn't be blocked by autonomy
    let result = tool.execute(json!({"operation": "status"})).await.unwrap();
    // The error should be about git (not about autonomy/read-only mode)
    assert!(result.is_error, "Expected failure due to missing git repo");
    let error_msg = result.output();
    assert!(
        !error_msg.is_empty(),
        "Expected a git-related error message"
    );
    assert!(
        !error_msg.contains("read-only") && !error_msg.contains("autonomy"),
        "Error should be about git, not about autonomy restrictions: {error_msg}"
    );
}

#[tokio::test]
async fn rejects_missing_operation() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());

    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'operation'"));
}

#[tokio::test]
async fn rejects_unknown_operation() {
    let tmp = TempDir::new().unwrap();
    // Initialize a git repository
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let tool = test_tool(tmp.path());

    let result = tool.execute(json!({"operation": "push"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Unknown operation"));
}

#[test]
fn truncates_multibyte_commit_message_without_panicking() {
    let long = "🦀".repeat(2500);
    let truncated = GitOperationsTool::truncate_commit_message(&long);

    assert_eq!(truncated.chars().count(), 2000);
}

// ── truncate_commit_message: short messages pass through unchanged ─────────

#[test]
fn truncate_short_message_unchanged() {
    let msg = "Fix the bug";
    assert_eq!(GitOperationsTool::truncate_commit_message(msg), msg);
}

#[test]
fn truncate_exact_2000_chars_unchanged() {
    let msg = "a".repeat(2000);
    let result = GitOperationsTool::truncate_commit_message(&msg);
    assert_eq!(result.chars().count(), 2000);
    assert!(!result.ends_with("..."));
}

#[test]
fn truncate_2001_chars_adds_ellipsis() {
    let msg = "a".repeat(2001);
    let result = GitOperationsTool::truncate_commit_message(&msg);
    assert!(result.ends_with("..."));
    assert_eq!(result.chars().count(), 2000);
}

// ── sanitize_git_args: allow leading dash that is not -c ─────────────────

#[test]
fn sanitize_git_allows_other_flags() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());
    assert!(tool.sanitize_git_args("--follow").is_ok());
    assert!(tool.sanitize_git_args("-p").is_ok());
    assert!(tool.sanitize_git_args("-n5").is_ok());
}

// ── requires_write_access completeness ────────────────────────────────────

#[test]
fn requires_write_access_covers_all_write_ops() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());
    for op in ["commit", "add", "checkout", "stash", "reset", "revert"] {
        assert!(
            tool.requires_write_access(op),
            "'{op}' should require write access"
        );
    }
}

// ── schema validation ─────────────────────────────────────────────────────

#[test]
fn schema_has_required_operation() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(
        required.contains(&serde_json::json!("operation")),
        "schema required should include 'operation'"
    );
}

#[test]
fn schema_enumerates_operations() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());
    let schema = tool.parameters_schema();
    let ops = schema["properties"]["operation"]["enum"]
        .as_array()
        .unwrap();
    let op_names: Vec<&str> = ops.iter().map(|v| v.as_str().unwrap()).collect();
    for expected in [
        "status", "diff", "log", "branch", "commit", "add", "checkout", "stash",
    ] {
        assert!(
            op_names.contains(&expected),
            "schema should include '{expected}'"
        );
    }
}

// ── git_operations tool name / description ────────────────────────────────

#[test]
fn tool_name_and_description() {
    let tmp = TempDir::new().unwrap();
    let tool = test_tool(tmp.path());
    assert_eq!(tool.name(), "git_operations");
    assert!(!tool.description().is_empty());
    assert!(tool.description().contains("Git"));
}

// ── not_in_git_repo returns error (covers the git-repo check) ─────────────

#[tokio::test]
async fn not_in_git_repo_returns_error() {
    let tmp = TempDir::new().unwrap();
    // Do NOT init a git repo
    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Not in a git repository"));
}

/// Initialise a git repo at `path` and fail the test if `git init`
/// itself didn't succeed (so we don't misread later assertion failures
/// as product bugs when the real problem is a missing/broken git).
fn init_git_repo(path: &std::path::Path) {
    let output = std::process::Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .expect("failed to spawn `git init`");
    assert!(
        output.status.success(),
        "`git init` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Extract the error text from a Result<ToolResult> — whether the
/// failure came through `Err(anyhow::Error)` or `Ok(ToolResult::error)`.
fn error_text(result: &anyhow::Result<ToolResult>) -> String {
    match result {
        Ok(r) => {
            assert!(r.is_error, "expected a tool-error ToolResult");
            r.output().to_string()
        }
        Err(e) => e.to_string(),
    }
}

// ── stash: unknown action returns error ────────────────────────────────────

#[tokio::test]
async fn stash_unknown_action_returns_error() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    let tool = test_tool(tmp.path());
    let result = tool
        .execute(json!({"operation": "stash", "action": "squash"}))
        .await;
    let msg = error_text(&result);
    assert!(
        msg.contains("Unknown stash action"),
        "expected 'Unknown stash action' in error, got: {msg}"
    );
}

// ── checkout: dangerous characters ────────────────────────────────────────

#[tokio::test]
async fn checkout_rejects_dangerous_branch_names() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    let tool = test_tool(tmp.path());

    for dangerous in ["main@{1}", "HEAD^", "v1~2"] {
        let result = tool
            .execute(json!({"operation": "checkout", "branch": dangerous}))
            .await;
        let msg = error_text(&result);
        assert!(
            msg.contains("invalid characters") || msg.contains("Invalid branch"),
            "expected a dangerous-branch rejection for '{dangerous}', got: {msg}"
        );
    }
}

// ── commit: missing message ────────────────────────────────────────────────

#[tokio::test]
async fn commit_missing_message_returns_error() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "commit"})).await;
    let msg = error_text(&result);
    assert!(
        msg.contains("Missing 'message' parameter"),
        "expected missing-message error, got: {msg}"
    );
}

// ── add: missing paths ─────────────────────────────────────────────────────

#[tokio::test]
async fn add_missing_paths_returns_error() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "add"})).await;
    let msg = error_text(&result);
    assert!(
        msg.contains("Missing 'paths' parameter"),
        "expected missing-paths error, got: {msg}"
    );
}
