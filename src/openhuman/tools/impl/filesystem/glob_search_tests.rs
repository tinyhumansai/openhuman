use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy, TrustedAccess, TrustedRoot};

fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        action_dir: workspace.clone(),
        workspace_dir: workspace.clone(),
        // Mirror the production constructor, which registers the action
        // sandbox as a ReadWrite trusted root so validate_path accepts it
        // even under workspace_only.
        trusted_roots: vec![TrustedRoot {
            path: workspace.to_string_lossy().to_string(),
            access: TrustedAccess::ReadWrite,
        }],
        ..SecurityPolicy::default()
    })
}

/// Policy with a distinct action sandbox and internal workspace — the real
/// production shape, and the configuration that surfaced #3357.
fn test_security_split(
    action_dir: std::path::PathBuf,
    workspace_dir: std::path::PathBuf,
    extra_roots: Vec<std::path::PathBuf>,
) -> Arc<SecurityPolicy> {
    let mut roots = vec![TrustedRoot {
        path: action_dir.to_string_lossy().to_string(),
        access: TrustedAccess::ReadWrite,
    }];
    roots.extend(extra_roots.into_iter().map(|p| TrustedRoot {
        path: p.to_string_lossy().to_string(),
        access: TrustedAccess::ReadWrite,
    }));
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        action_dir,
        workspace_dir,
        trusted_roots: roots,
        ..SecurityPolicy::default()
    })
}

#[test]
fn glob_name() {
    let tool = GlobTool::new(test_security(std::env::temp_dir()));
    assert_eq!(tool.name(), "glob");
}

#[tokio::test]
async fn glob_matches_extension() {
    let dir = std::env::temp_dir().join("openhuman_test_glob_ext");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(dir.join("src/sub"))
        .await
        .unwrap();
    tokio::fs::write(dir.join("src/a.rs"), "// a")
        .await
        .unwrap();
    tokio::fs::write(dir.join("src/sub/b.rs"), "// b")
        .await
        .unwrap();
    tokio::fs::write(dir.join("src/c.txt"), "c").await.unwrap();

    let tool = GlobTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"pattern": "**/*.rs"})).await.unwrap();
    assert!(!result.is_error);
    let output = result.output();
    assert!(output.contains("src/a.rs"));
    assert!(output.contains("src/sub/b.rs"));
    assert!(!output.contains("c.txt"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn glob_invalid_pattern() {
    let dir = std::env::temp_dir().join("openhuman_test_glob_invalid");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let tool = GlobTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"pattern": "**["})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Invalid glob"));
    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn glob_skips_node_modules() {
    let dir = std::env::temp_dir().join("openhuman_test_glob_skip");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(dir.join("node_modules"))
        .await
        .unwrap();
    tokio::fs::write(dir.join("node_modules/lib.js"), "")
        .await
        .unwrap();
    tokio::fs::write(dir.join("app.js"), "").await.unwrap();

    let tool = GlobTool::new(test_security(dir.clone()));
    let result = tool.execute(json!({"pattern": "**/*.js"})).await.unwrap();
    let output = result.output();
    assert!(output.contains("app.js"));
    assert!(!output.contains("node_modules"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

/// Regression for #3357: glob roots at action_dir (so its hits are readable),
/// and never surfaces files living under the internal workspace_dir.
#[tokio::test]
async fn glob_roots_at_action_dir_and_excludes_workspace() {
    let root = std::env::temp_dir().join("openhuman_test_glob_split");
    let action = root.join("action");
    let workspace = root.join("workspace");
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&action).await.unwrap();
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::write(action.join("keep.txt"), "keep")
        .await
        .unwrap();
    tokio::fs::write(workspace.join("secret.txt"), "secret")
        .await
        .unwrap();

    let security = test_security_split(action.clone(), workspace.clone(), vec![]);
    let tool = GlobTool::new(security.clone());
    let result = tool.execute(json!({"pattern": "**/*.txt"})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let output = result.output();
    // Action-sandbox file is found, rendered relative...
    assert!(output.contains("keep.txt"), "missing keep.txt: {output}");
    // ...and the internal workspace file is NOT enumerated.
    assert!(
        !output.contains("secret.txt"),
        "leaked workspace file: {output}"
    );

    // The glob hit must be directly readable by the reader tools.
    assert!(
        security.validate_path("keep.txt").await.is_ok(),
        "glob hit not resolvable by validate_path"
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}

/// "As wide as the readers": glob can search any granted trusted root via
/// the `path` arg, returning absolute paths that the readers accept as-is.
#[tokio::test]
async fn glob_searches_named_trusted_root() {
    let root = std::env::temp_dir().join("openhuman_test_glob_trusted");
    let action = root.join("action");
    let workspace = root.join("workspace");
    let granted = root.join("granted");
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&action).await.unwrap();
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::create_dir_all(&granted).await.unwrap();
    tokio::fs::write(granted.join("data.csv"), "x")
        .await
        .unwrap();

    let security = test_security_split(action.clone(), workspace.clone(), vec![granted.clone()]);
    let tool = GlobTool::new(security.clone());
    let granted_abs = granted.to_string_lossy().to_string();
    let result = tool
        .execute(json!({"pattern": "**/*.csv", "path": granted_abs}))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let output = result.output();
    assert!(output.contains("data.csv"), "missing data.csv: {output}");
    // Rendered absolute (outside the action sandbox).
    assert!(
        output.contains(&granted.to_string_lossy().to_string()),
        "expected absolute path: {output}"
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}

/// A search root the policy disallows yields a clear error, not ENOENT.
#[tokio::test]
async fn glob_rejects_disallowed_search_path() {
    let root = std::env::temp_dir().join("openhuman_test_glob_reject");
    let action = root.join("action");
    let workspace = root.join("workspace");
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&action).await.unwrap();
    tokio::fs::create_dir_all(&workspace).await.unwrap();

    let security = test_security_split(action.clone(), workspace.clone(), vec![]);
    let tool = GlobTool::new(security);
    // Absolute path outside any trusted root, under workspace_only.
    let result = tool
        .execute(json!({"pattern": "**/*", "path": "/etc"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("not accessible"),
        "{}",
        result.output()
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}

/// Security backstop: a symlink inside the sandbox pointing OUTSIDE it must
/// never leak the target. Two layers cover this and this test exercises
/// both: the walk runs with `follow_links(false)`, so the symlink entry is
/// dropped by the `is_file()` gate (a symlink is not a regular file) and is
/// never descended; and `is_path_string_allowed` is the fail-closed per-hit
/// backstop — assert directly that it rejects the escape's resolved string,
/// since that is the check `collect_matches` leans on if the walk gate is
/// ever bypassed. A legitimate in-sandbox file is still found.
#[cfg(unix)]
#[tokio::test]
async fn glob_does_not_leak_symlink_escape() {
    let root = std::env::temp_dir().join("openhuman_test_glob_symlink");
    let action = root.join("action");
    let workspace = root.join("workspace");
    let outside = root.join("outside");
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(action.join("sub")).await.unwrap();
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::create_dir_all(&outside).await.unwrap();
    tokio::fs::write(action.join("sub/ok.txt"), "ok")
        .await
        .unwrap();
    tokio::fs::write(outside.join("secret.txt"), "secret")
        .await
        .unwrap();
    // Symlink inside the sandbox pointing at the outside (untrusted) tree.
    std::os::unix::fs::symlink(&outside, action.join("escape")).unwrap();

    // `outside` is deliberately NOT registered as a trusted root.
    let security = test_security_split(action.clone(), workspace.clone(), vec![]);
    let tool = GlobTool::new(security.clone());
    let result = tool.execute(json!({"pattern": "**/*.txt"})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let output = result.output();
    // The in-sandbox file is found, the escape target is not enumerated.
    assert!(
        output.contains("ok.txt"),
        "missing in-sandbox file: {output}"
    );
    assert!(
        !output.contains("secret"),
        "leaked symlink-escape target: {output}"
    );
    // The fail-closed backstop, exercised directly: the resolved escape
    // path is rejected as a string regardless of how it was reached.
    let escaped = outside.join("secret.txt").to_string_lossy().to_string();
    assert!(
        !security.is_path_string_allowed(&escaped),
        "policy must reject the escape target string: {escaped}"
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}

/// A `path` that resolves to a *file* (not a directory) yields a clear
/// "not a directory" error rather than a misleading "0 match(es)".
#[tokio::test]
async fn glob_rejects_file_search_path() {
    let dir = std::env::temp_dir().join("openhuman_test_glob_file_root");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("file.txt"), "x").await.unwrap();

    let tool = GlobTool::new(test_security(dir.clone()));
    let result = tool
        .execute(json!({"pattern": "**/*", "path": "file.txt"}))
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output());
    assert!(
        result.output().contains("not a directory"),
        "{}",
        result.output()
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}
