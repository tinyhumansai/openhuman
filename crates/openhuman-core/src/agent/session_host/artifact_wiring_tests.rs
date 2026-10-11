use super::*;
use crate::security::policy::tool_result_artifacts_dir;

const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

fn age(path: &Path, by: std::time::Duration) {
    let old = std::time::SystemTime::now() - by;
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(old).unwrap();
}

#[cfg(windows)]
fn open_directory(path: &Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::File::options()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

#[cfg(not(windows))]
fn open_directory(path: &Path) -> std::fs::File {
    std::fs::File::open(path).unwrap()
}

fn stale_session(root: &Path, session: &str) -> PathBuf {
    let dir = root.join(session).join("shell");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("call.txt");
    std::fs::write(&file, "old output").unwrap();
    age(&file, 2 * DAY);
    // A directory's own mtime counts toward freshness too.
    for d in [dir.as_path(), dir.parent().unwrap()] {
        let handle = open_directory(d);
        handle
            .set_modified(std::time::SystemTime::now() - 2 * DAY)
            .unwrap();
    }
    root.join(session)
}

/// The store writes under the workspace, never the action directory, whether
/// with absolute pointers independent of the turn's working directory.
#[test]
fn the_store_is_rooted_in_the_workspace_not_the_action_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let store = build_artifact_store(&workspace, "s");
    assert!(store.is_detached());
    assert_eq!(store.root(), tool_result_artifacts_dir(&workspace));
    let pointer = store.path_for_read_tool("shell", Some("c"));
    assert!(Path::new(&pointer).starts_with(workspace.join("artifacts/tool-results")));
}

/// Workspace retention never deletes files from the agent's project after
/// retiring the old action-directory migration sweep.
#[test]
fn workspace_retention_leaves_legacy_project_artifacts_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let action = tmp.path().join("project");
    let legacy_root = action.join("artifacts/tool-results");
    let stale = stale_session(&legacy_root, "old");
    let fresh = legacy_root.join("recent/shell");
    std::fs::create_dir_all(&fresh).unwrap();
    std::fs::write(fresh.join("call.txt"), "recent").unwrap();
    let detached_stale = stale_session(&tool_result_artifacts_dir(&workspace), "older");

    build_artifact_store(&workspace, "current");

    assert!(
        stale.exists(),
        "project files are outside workspace retention"
    );
    assert!(fresh.join("call.txt").exists(), "a fresh one is kept");
    assert!(!detached_stale.exists(), "the workspace store is swept too");
}
