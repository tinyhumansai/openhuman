use super::*;

// -- canonical_workspace cache ------------------------------------

/// `validate_path` caches the canonical workspace root for repeated calls.
#[tokio::test]
async fn validate_path_caches_canonical_workspace_root() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    let file = workspace.join("hello.txt");
    std::fs::write(&file, "hi").unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    assert!(policy.canonical_workspace.get().is_none());
    let r1 = policy.validate_path(file.to_str().unwrap()).await.unwrap();
    let cached = policy.canonical_workspace.get().unwrap().clone();
    for _ in 0..5 {
        assert_eq!(
            policy.validate_path(file.to_str().unwrap()).await.unwrap(),
            r1
        );
        assert_eq!(policy.canonical_workspace.get(), Some(&cached));
    }
}

/// The synchronous and asynchronous workspace-root helpers share one cache.
#[tokio::test]
async fn workspace_root_sync_hydrates_and_shares_the_async_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    let expected = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.clone());
    let policy = SecurityPolicy {
        workspace_dir: workspace,
        action_dir: tmp.path().to_path_buf(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    assert!(policy.canonical_workspace.get().is_none());
    let r1 = policy.workspace_root_sync();
    assert_eq!(r1, expected);
    assert_eq!(policy.canonical_workspace.get(), Some(&expected));
    for _ in 0..5 {
        assert_eq!(policy.workspace_root_sync(), r1);
    }
    assert_eq!(policy.workspace_root().await, r1);
    assert!(policy.is_resolved_path_allowed(&expected.join("note.txt")));
}

/// `validate_parent_path` and `validate_path` use the same workspace cache.
#[tokio::test]
async fn validate_parent_path_uses_same_cache_as_validate_path() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    assert!(policy.canonical_workspace.get().is_none());
    policy
        .validate_parent_path(workspace.join("new.txt").to_str().unwrap())
        .await
        .unwrap();
    let cached = policy.canonical_workspace.get().unwrap().clone();
    std::fs::write(workspace.join("hi.txt"), "x").unwrap();
    policy
        .validate_path(workspace.join("hi.txt").to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(policy.canonical_workspace.get(), Some(&cached));
}

#[cfg(unix)]
#[tokio::test]
async fn missing_workspace_does_not_cache_unresolved_symlink_spelling() {
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let link = parent.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let workspace = link.join("workspace");
    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };

    let err = policy.validate_parent_path("new.txt").await.unwrap_err();
    assert!(err.contains(WORKSPACE_MISSING_MARKER), "err: {err}");
    assert!(policy.canonical_workspace.get().is_none());

    std::fs::create_dir(&workspace).unwrap();
    assert!(policy.validate_parent_path("new.txt").await.is_ok());
    let canonical_workspace = workspace.canonicalize().unwrap();
    assert_eq!(policy.canonical_workspace.get(), Some(&canonical_workspace));
}

#[cfg(unix)]
#[tokio::test]
async fn missing_workspace_under_protected_symlink_is_not_repairable() {
    let parent = tempfile::tempdir().unwrap();
    let link = parent.path().join("link");
    std::os::unix::fs::symlink("/etc", &link).unwrap();
    let workspace = link.join("missing-workspace");
    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };

    let err = policy.validate_parent_path("new.txt").await.unwrap_err();
    assert!(!err.contains(WORKSPACE_MISSING_MARKER), "err: {err}");
    assert!(err.contains("protected"), "err: {err}");
}
