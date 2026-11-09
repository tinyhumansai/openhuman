//! A build without the `security-module` feature has security disabled
//! (`crate::security::switch`): every policy gate allows instead of failing
//! closed.

use std::path::Path;

use super::types::SecurityPolicy;

#[tokio::test]
async fn missing_tinysecurity_allows_the_protected_floor_and_resolves_paths_locally() {
    assert!(
        !SecurityPolicy::is_always_forbidden_async(Path::new("/tmp/protected-test")).await,
        "a build without the module has no protected-path floor"
    );

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.md"), "notes").unwrap();
    let policy = SecurityPolicy {
        workspace_dir: dir.path().to_path_buf(),
        action_dir: dir.path().to_path_buf(),
        ..SecurityPolicy::default()
    };
    let resolved = policy
        .validate_path("notes.md")
        .await
        .expect("file access needs no TinySecurity authorization when it is not built");
    assert_eq!(
        resolved,
        dir.path().join("notes.md").canonicalize().unwrap()
    );
}

#[tokio::test]
async fn a_build_without_the_module_allows_every_policy_gate() {
    super::tests::assert_every_policy_gate_allows().await;
}
