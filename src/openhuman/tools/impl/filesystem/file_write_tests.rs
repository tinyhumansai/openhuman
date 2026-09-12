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

fn test_security_with(
    workspace: std::path::PathBuf,
    autonomy: AutonomyLevel,
    max_actions_per_hour: u32,
) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy,
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        max_actions_per_hour,
        ..SecurityPolicy::default()
    })
}

#[test]
fn file_write_name() {
    let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
    assert_eq!(tool.name(), "file_write");
}

#[test]
fn file_write_schema_has_path_and_content() {
    let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["path"].is_object());
    assert!(schema["properties"]["content"].is_object());
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("path")));
    assert!(required.contains(&json!("content")));
}

#[test]
fn approval_probe_uses_effective_workspace_root() {
    let root = tempfile::tempdir().expect("root");
    let profile_workspace = root.path().join("profiles/alice");
    std::fs::create_dir_all(&profile_workspace).expect("profile workspace");
    std::fs::write(profile_workspace.join("notes.md"), "existing").expect("seed file");

    let tool = FileWriteTool::with_approval_workspace_root(
        test_security(root.path().to_path_buf()),
        profile_workspace,
    );

    assert!(tool.external_effect_with_args(&json!({
        "path": "notes.md",
        "content": "updated"
    })));
}

#[tokio::test]
async fn file_write_creates_file() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"path": "out.txt", "content": "written!"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("8 bytes"));

    let content = tokio::fs::read_to_string(dir.join("out.txt"))
        .await
        .unwrap();
    assert_eq!(content, "written!");

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn file_write_creates_parent_dirs() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_nested");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"path": "a/b/c/deep.txt", "content": "deep"}))
        .await
        .unwrap();
    assert!(!result.is_error);

    let content = tokio::fs::read_to_string(dir.join("a/b/c/deep.txt"))
        .await
        .unwrap();
    assert_eq!(content, "deep");

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn file_write_overwrites_existing() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_overwrite");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("exist.txt"), "old")
        .await
        .unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"path": "exist.txt", "content": "new"}))
        .await
        .unwrap();
    assert!(!result.is_error);

    let content = tokio::fs::read_to_string(dir.join("exist.txt"))
        .await
        .unwrap();
    assert_eq!(content, "new");

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn file_write_blocks_path_traversal() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_traversal");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"path": "../../etc/evil", "content": "bad"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(&result.output().contains("not allowed"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn file_write_blocks_absolute_path() {
    let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
    let result = tool
        .execute(json!({"path": "/etc/evil", "content": "bad"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(&result.output().contains("not allowed"));
}

#[tokio::test]
async fn file_write_missing_path_param() {
    let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
    let result = tool.execute(json!({"content": "data"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn file_write_missing_content_param() {
    let tool = FileWriteTool::new(test_security(std::env::temp_dir()));
    let result = tool.execute(json!({"path": "file.txt"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn file_write_empty_content() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_empty");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"path": "empty.txt", "content": ""}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("0 bytes"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[cfg(unix)]
#[tokio::test]
async fn file_write_blocks_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join("openhuman_test_file_write_symlink_escape");
    let workspace = root.join("workspace");
    let outside = root.join("outside");

    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::create_dir_all(&outside).await.unwrap();

    symlink(&outside, workspace.join("escape_dir")).unwrap();

    let tool = FileWriteTool::new(test_security(workspace.clone()));
    let result = tool
        .execute(json!({"path": "escape_dir/hijack.txt", "content": "bad"}))
        .await
        .unwrap();

    assert!(result.is_error);
    // SecurityPolicy now blocks symlink escapes at the is_path_allowed
    // layer (#1927) — error becomes "Path not allowed by security
    // policy" rather than the deeper "escapes workspace" message.
    let out = result.output();
    assert!(
        out.contains("escapes workspace") || out.contains("not allowed"),
        "expected escape/not-allowed error, got: {out}"
    );
    assert!(!outside.join("hijack.txt").exists());

    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn file_write_blocks_readonly_mode() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_readonly");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security_with(dir.clone(), AutonomyLevel::ReadOnly, 20));
    let result = tool
        .execute(json!({"path": "out.txt", "content": "should-block"}))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
    // The readonly block must carry the hard-reject marker so the agent
    // harness recognizes it and halts on a verbatim repeat instead of
    // grinding. Ties this tool's literal to the marker const — the
    // const→detector half lives in `RepeatedToolFailureMiddleware` and is
    // covered by its tests (`src/openhuman/agent/tinyagents/middleware.rs`).
    assert!(
        result
            .output()
            .contains(crate::openhuman::security::POLICY_BLOCKED_MARKER),
        "file_write readonly block must carry the hard-reject marker: {}",
        result.output()
    );
    assert!(!dir.join("out.txt").exists());

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn file_write_blocks_when_rate_limited() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_rate_limited");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security_with(
        dir.clone(),
        AutonomyLevel::Supervised,
        0,
    ));
    let result = tool
        .execute(json!({"path": "out.txt", "content": "should-block"}))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("Rate limit exceeded"));
    assert!(!dir.join("out.txt").exists());

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

// ── §5.1 TOCTOU / symlink file write protection tests ────

#[cfg(unix)]
#[tokio::test]
async fn file_write_blocks_symlink_target_file() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join("openhuman_test_file_write_symlink_target");
    let workspace = root.join("workspace");
    let outside = root.join("outside");

    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::create_dir_all(&outside).await.unwrap();

    // Create a file outside and symlink to it inside workspace
    tokio::fs::write(outside.join("target.txt"), "original")
        .await
        .unwrap();
    symlink(outside.join("target.txt"), workspace.join("linked.txt")).unwrap();

    let tool = FileWriteTool::new(test_security(workspace.clone()));
    let result = tool
        .execute(json!({"path": "linked.txt", "content": "overwritten"}))
        .await
        .unwrap();

    assert!(result.is_error, "writing through symlink must be blocked");
    // The symlink-safe is_path_allowed check (#1927) blocks at the
    // policy layer before the tool's own symlink-target detection
    // runs; accept either error message.
    let out = result.output();
    assert!(
        out.contains("symlink") || out.contains("not allowed"),
        "error should mention symlink or policy block, got: {out}"
    );

    // Verify original file was not modified
    let content = tokio::fs::read_to_string(outside.join("target.txt"))
        .await
        .unwrap();
    assert_eq!(content, "original", "original file must not be modified");

    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn file_write_blocks_null_byte_in_path() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_null");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"path": "file\u{0000}.txt", "content": "bad"}))
        .await
        .unwrap();
    assert!(result.is_error, "paths with null bytes must be blocked");

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

/// `file_write` enforces the same 5MB content-size cap as `edit_file` and
/// `grep`'s file caps (`file_read` allows up to 10MB for reads).
#[tokio::test]
async fn file_write_reports_a_refused_write_rather_than_a_silent_success() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_refused");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone())).with_sink(Arc::new(
        super::super::write_sink::RefusingSink(std::io::ErrorKind::PermissionDenied),
    ));
    let result = tool
        .execute(json!({"path": "f.txt", "content": "abc"}))
        .await
        .unwrap();

    assert!(
        result.is_error,
        "a refused write must surface as an error, not a fabricated success"
    );
    assert!(
        !dir.join("f.txt").exists(),
        "and nothing may be left behind claiming the write happened"
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn file_write_refuses_an_oversized_content_write() {
    let dir = std::env::temp_dir().join("openhuman_test_file_write_oversized");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let tool = FileWriteTool::new(test_security(dir.clone()));
    let content = "x".repeat(64 * 1024 * 1024);
    let result = tool
        .execute(json!({"path": "huge.txt", "content": content}))
        .await
        .unwrap();
    assert!(
        result.is_error,
        "a 64MB write must be refused by a stated ceiling, not written: {}",
        result.output()
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}
