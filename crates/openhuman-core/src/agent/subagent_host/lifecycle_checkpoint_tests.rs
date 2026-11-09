use super::*;

/// The in-flight map is keyed by this path, so it must name a directory the
/// same way before and after the lifecycle store creates it. Reached through a
/// symlinked parent (macOS `/var` -> `/private/var`), a path that resolved
/// literally while absent and canonically once present split one task into two
/// leaders.
#[cfg(unix)]
#[test]
fn checkpoint_dir_key_is_stable_across_directory_creation() {
    let scratch = tempfile::tempdir().expect("scratch");
    let real = scratch.path().join("real");
    std::fs::create_dir_all(&real).expect("real parent");
    let link = scratch.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symlinked parent");

    let requested = link.join(".openhuman/subagent_checkpoints");
    let before = absolute_checkpoint_dir(requested.clone());
    std::fs::create_dir_all(&requested).expect("create checkpoint dir");
    let after = absolute_checkpoint_dir(requested);

    assert_eq!(
        before, after,
        "key must not change when the directory appears"
    );
    assert_eq!(after, std::fs::canonicalize(&after).expect("exists"));
}

#[test]
fn relative_checkpoint_dir_is_made_absolute() {
    let resolved = absolute_checkpoint_dir(PathBuf::from("does/not/exist/yet"));
    assert!(resolved.is_absolute());
    assert!(resolved.ends_with("does/not/exist/yet"));
}
