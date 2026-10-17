use super::*;
use crate::security::policy::tool_result_artifacts_dir;
use serde_json::json;
use tinyagents_harness::artifacts::tool_results::{apply_per_result_persistence, ArtifactRead};

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

fn stale_session(root: &Path, session: &str) -> std::path::PathBuf {
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

/// The store materialises under the workspace, never the action directory, and
/// hands the model a blob key rather than a path.
#[tokio::test]
async fn the_store_materialises_in_the_workspace_not_the_action_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let store = build_artifact_store(&workspace, "s");
    assert_eq!(
        store.materialize_root(),
        tool_result_artifacts_dir(&workspace)
    );
    let pointer = store.path_for_read_tool("shell", Some("c"));
    assert_eq!(pointer, "tool-results/s/shell/c.txt");
}

/// A large result is stored on the blob store, its pointer opens through the
/// materialised file, and a rebuilt store (a session reload) still reads it.
#[tokio::test]
async fn a_stored_result_survives_a_session_reload() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let first = build_artifact_store(&workspace, "thread-1");
    let raw = "z".repeat(4096);
    let (out, outcome) =
        apply_per_result_persistence(raw, None, Some(&first), "shell", Some("call-1"), 1024).await;
    assert!(outcome.persisted, "{out}");
    let key = outcome.artifact_path.expect("pointer");
    assert!(key.starts_with("tool-results/thread-1/shell/"), "{key}");

    let reloaded = build_artifact_store(&workspace, "thread-1");
    let mut args = json!({"path": key});
    reloaded
        .resolve_read(
            &mut args,
            &ArtifactRead {
                path: key.clone(),
                offset: 0,
            },
        )
        .await;
    let path = args["path"].as_str().unwrap();
    assert!(Path::new(path).starts_with(tool_result_artifacts_dir(&workspace)));
    assert!(std::fs::read_to_string(path).unwrap().contains("zzzz"));
}

/// Retention sweeps stale materialised copies and nothing in the project.
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
    let materialised_stale = stale_session(&tool_result_artifacts_dir(&workspace), "older");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();
    build_artifact_store(&workspace, "current");

    assert!(
        stale.exists(),
        "project files are outside workspace retention"
    );
    assert!(fresh.join("call.txt").exists(), "a fresh one is kept");
    assert!(
        !materialised_stale.exists(),
        "the workspace cache is swept too"
    );
}
