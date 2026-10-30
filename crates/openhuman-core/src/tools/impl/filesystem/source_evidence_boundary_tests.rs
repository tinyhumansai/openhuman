use super::*;

#[tokio::test]
async fn traversal_and_absolute_out_of_scope_paths_are_rejected() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("outside.rs"), "secret").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    assert!(tool
        .execute(json!({"operation":"scan","path":"../"}))
        .await
        .is_err());
    assert!(tool
        .execute(json!({
            "operation":"verify","path":outside.path().join("outside.rs").to_string_lossy(),
            "expected_sha256":"0".repeat(64)
        }))
        .await
        .is_err());
}

#[tokio::test]
async fn skips_secret_files_ignored_roots_and_symbolic_links() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("ok.rs"), "visible").unwrap();
    std::fs::write(project.path().join(".env"), "PRIVATE_TOKEN=token").unwrap();
    std::fs::create_dir(project.path().join(".git")).unwrap();
    std::fs::write(project.path().join(".git/config"), "secret git config").unwrap();
    std::fs::write(outside.path().join("secret.rs"), "outside private").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("secret.rs"),
        project.path().join("link.rs"),
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let envelope = run(&tool, json!({"operation":"scan","query":"private"})).await;
    let payload = envelope.to_string();
    assert!(payload.contains("ok.rs"));
    assert!(!payload.contains("PRIVATE_TOKEN"));
    assert!(!payload.contains("outside private"));
    assert!(!payload.contains("secret.rs"));
    assert!(!payload.contains(".git/config"));
    assert!(!payload.contains(".env"));
}

#[tokio::test]
async fn workspace_descriptor_grants_only_its_root_per_call() {
    let home = tempfile::tempdir().unwrap();
    let ungranted = tempfile::tempdir().unwrap();
    let descriptor_root = tempfile::tempdir().unwrap();
    std::fs::write(descriptor_root.path().join("code.rs"), "fn scoped() {}").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), ungranted.path()));
    let context = TestWorkspace(
        WorkspaceDescriptor::new(descriptor_root.path().to_path_buf())
            .with_policy_id("test-evidence"),
    );
    let result = tool
        .execute_with_context(
            json!({"operation":"scan","path":"."}),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .expect("descriptor root authorized");
    assert!(result.output().contains("code.rs"));
    assert!(!result.output().contains(ungranted.path().to_str().unwrap()));
}

#[tokio::test]
async fn traversal_cannot_be_smuggled_as_pagination_marker() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("safe.rs"), "fn safe() {}").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let invalid = tool
        .execute(json!({
            "operation":"scan","after_path":"../etc/passwd"
        }))
        .await;
    assert!(invalid.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn explicitly_addressed_symlink_file_cannot_bypass_scan_exclusion() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("target.rs"), "fn target() {}\n").unwrap();
    std::os::unix::fs::symlink(
        project.path().join("target.rs"),
        project.path().join("link.rs"),
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    assert!(tool
        .execute(json!({"operation":"scan","path":"link.rs"}))
        .await
        .is_err());
}

#[tokio::test]
async fn explicit_sensitive_ancestor_paths_are_excluded_even_when_file_basename_is_safe() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".git")).unwrap();
    std::fs::write(
        project.path().join(".git/config"),
        "fake-token = never-return",
    )
    .unwrap();
    std::fs::create_dir(project.path().join("node_modules")).unwrap();
    std::fs::write(
        project.path().join("node_modules/notes.rs"),
        "fake-token = never-return",
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    for requested in [".git/config", "node_modules/notes.rs"] {
        assert!(
            tool.execute(json!({"operation":"scan","path":requested}))
                .await
                .is_err(),
            "must reject explicitly targeted excluded ancestor {requested}"
        );
        assert!(tool
            .execute(json!({
                "operation":"verify", "path":requested,
                "expected_sha256":"0".repeat(64)
            }))
            .await
            .is_err());
    }
}

#[tokio::test]
async fn sensitive_directory_exclusion_is_case_insensitive_and_blocks_nested_files() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for directory in [".GIT", "NODE_MODULES", ".CACHE-build"] {
        let folder = project.path().join(directory);
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("payload.rs"), "fn private_code() {}").unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let root_scan = run(&tool, json!({"operation":"scan","path":"."})).await;
    assert_eq!(root_scan["files"].as_array().unwrap().len(), 0);
    for directory in [".GIT", "NODE_MODULES", ".CACHE-build"] {
        let target = format!("{directory}/payload.rs");
        assert!(
            tool.execute(json!({"operation":"scan","path":target}))
                .await
                .is_err(),
            "case-insensitive deny boundary must reject {target}"
        );
    }
}

#[tokio::test]
async fn hidden_cloud_and_machine_credential_directories_are_never_indexed() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for directory in [".ssh", ".aws", ".kube", ".azure", ".gnupg", ".docker"] {
        let folder = project.path().join(directory);
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("config"), "DO_NOT_SURFACE_CREDENTIAL").unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(&tool, json!({"operation":"scan","query":"DO_NOT_SURFACE"})).await;
    assert_eq!(result["files"].as_array().unwrap().len(), 0);
    for directory in [".ssh", ".aws", ".kube", ".azure", ".gnupg", ".docker"] {
        let candidate = format!("{directory}/config");
        assert!(
            tool.execute(json!({"operation":"scan","path":candidate}))
                .await
                .is_err(),
            "must reject explicit machine-credential directory: {candidate}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn unix_literal_backslashes_do_not_collide_with_separator_paths() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/file.rs"), "fn nested() {}\n").unwrap();
    std::fs::write(
        project.path().join("src\\file.rs"),
        "fn literal_backslash() {}\n",
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));

    let result = run(&tool, json!({"operation":"scan"})).await;
    assert_eq!(result["files"].as_array().unwrap().len(), 2);
    let files = result["files"].as_array().unwrap();
    assert_ne!(files[0]["path"], files[1]["path"]);
    assert!(files.iter().any(|f| f["path"] == "src/file.rs"));
    assert!(files.iter().any(|f| f["path"] == "src\\file.rs"));

    let digest = files.iter().find(|f| f["path"] == "src\\file.rs").unwrap()["source_hash"]
        .as_str()
        .unwrap();
    let reread = run(
        &tool,
        json!({
            "operation": "verify", "path": "src\\file.rs",
            "expected_sha256": digest, "start_line": 1, "end_line": 1
        }),
    )
    .await;
    assert_eq!(reread["lines"][0]["text"], "fn literal_backslash() {}");
}

#[cfg(unix)]
#[tokio::test]
async fn non_utf8_filename_cannot_be_silently_counted_as_complete_scan() {
    use std::os::unix::ffi::OsStrExt;
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let non_utf8 = std::ffi::OsStr::from_bytes(b"source_\xff.rs");
    std::fs::write(project.path().join(non_utf8), "fn opaque() {}\n").unwrap();
    std::fs::write(project.path().join("good.rs"), "fn good() {}\n").unwrap();

    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(&tool, json!({"operation":"scan"})).await;
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    assert_eq!(result["files"][0]["path"], "good.rs");
    assert_eq!(result["complete"], false);
    assert_eq!(result["truncated_reason"], "non_utf8_path");
}

#[cfg(unix)]
#[tokio::test]
async fn unix_backslash_filename_must_remain_usable_as_safe_page_cursor() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/file.rs"), "fn directory() {}\n").unwrap();
    std::fs::write(project.path().join("src\\file.rs"), "fn literal() {}\n").unwrap();
    std::fs::write(project.path().join("t.rs"), "fn tail() {}\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let page = run(
        &tool,
        json!({
            "operation": "scan", "max_files": 1, "after_path": "src/file.rs"
        }),
    )
    .await;
    assert_eq!(page["files"][0]["path"], "src\\file.rs");
    assert_eq!(page["next_after_path"], "src\\file.rs");
    let final_page = run(
        &tool,
        json!({
            "operation": "scan", "max_files": 1, "after_path": page["next_after_path"]
        }),
    )
    .await;
    assert_eq!(final_page["files"][0]["path"], "t.rs");
    assert_eq!(final_page["complete"], true);
}

#[tokio::test]
async fn source_evidence_excludes_workspace_auth_material_without_hiding_auth_source_code() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".openhuman")).unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    for secret in [
        ".openhuman/credentials.toml",
        ".netrc",
        ".envrc",
        ".git-credentials",
        "auth.json",
        "token.json",
        "service-account.json",
    ] {
        std::fs::write(project.path().join(secret), "SOURCE_SECRET_DO_NOT_EXPOSE").unwrap();
    }
    std::fs::write(
        project.path().join("src/auth.ts"),
        "export function auth() { return true; }\n",
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let scan = run(
        &tool,
        json!({
            "operation":"scan", "query":"SOURCE_SECRET_DO_NOT_EXPOSE"
        }),
    )
    .await;
    assert_eq!(scan["complete"], true);
    assert_eq!(scan["files"].as_array().unwrap().len(), 1);
    assert_eq!(scan["files"][0]["path"], "src/auth.ts");
    assert!(!scan.to_string().contains("SOURCE_SECRET_DO_NOT_EXPOSE"));

    for secret in [
        ".openhuman/credentials.toml",
        ".netrc",
        ".envrc",
        ".git-credentials",
        "auth.json",
        "token.json",
        "service-account.json",
    ] {
        assert!(
            tool.execute(json!({"operation":"scan","path":secret}))
                .await
                .is_err(),
            "sensitive path must be refused: {secret}"
        );
    }
}
