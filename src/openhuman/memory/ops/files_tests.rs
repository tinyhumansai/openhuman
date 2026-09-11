use std::ffi::OsString;

use tempfile::TempDir;

use super::*;
use crate::openhuman::config::TEST_ENV_LOCK;

struct WorkspaceEnvGuard {
    previous: Option<OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self { previous }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = self.previous.take() {
                std::env::set_var("OPENHUMAN_WORKSPACE", previous);
            } else {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            }
        }
    }
}

#[tokio::test]
async fn write_read_and_list_memory_files_roundtrip() {
    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let write = ai_write_memory_file(WriteMemoryFileRequest {
        relative_path: "notes/today.md".to_string(),
        content: "remember this".to_string(),
    })
    .await
    .expect("write should succeed");
    let write_data = write.value.data.expect("write data");
    assert_eq!(write_data.relative_path, "notes/today.md");
    assert!(write_data.written);
    assert_eq!(write_data.bytes_written, "remember this".len());

    let read = ai_read_memory_file(ReadMemoryFileRequest {
        relative_path: "notes/today.md".to_string(),
    })
    .await
    .expect("read should succeed");
    let read_data = read.value.data.expect("read data");
    assert_eq!(read_data.relative_path, "notes/today.md");
    assert_eq!(read_data.content, "remember this");

    let memory_root = super::super::helpers::resolve_existing_memory_path("")
        .await
        .expect("resolve memory root");
    tokio::fs::write(memory_root.join("b.md"), "b")
        .await
        .expect("write b");
    tokio::fs::write(memory_root.join("a.md"), "a")
        .await
        .expect("write a");
    tokio::fs::create_dir_all(memory_root.join("nested"))
        .await
        .expect("create nested dir");
    tokio::fs::write(memory_root.join("nested").join("hidden.md"), "hidden")
        .await
        .expect("write nested file");

    let listed = ai_list_memory_files(ListMemoryFilesRequest {
        relative_dir: String::new(),
    })
    .await
    .expect("list should succeed");
    let listed_data = listed.value.data.expect("list data");
    assert_eq!(listed_data.relative_dir, "");
    assert_eq!(listed_data.files, vec!["a.md", "b.md"]);
    assert_eq!(listed_data.count, 2);
}

#[tokio::test]
async fn list_memory_files_skips_internal_sqlite_artifacts() {
    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let memory_root = super::super::helpers::resolve_existing_memory_path("")
        .await
        .expect("resolve memory root");
    tokio::fs::write(memory_root.join("memory.db"), "x")
        .await
        .expect("write db");
    tokio::fs::write(memory_root.join("memory.db-shm"), "x")
        .await
        .expect("write shm");
    tokio::fs::write(memory_root.join("memory.db-wal"), "x")
        .await
        .expect("write wal");
    tokio::fs::write(memory_root.join("visible.md"), "ok")
        .await
        .expect("write visible");

    let listed = ai_list_memory_files(ListMemoryFilesRequest {
        relative_dir: String::new(),
    })
    .await
    .expect("list should succeed");
    let listed_data = listed.value.data.expect("list data");
    assert_eq!(listed_data.files, vec!["visible.md"]);
    assert_eq!(listed_data.count, 1);
}

#[tokio::test]
async fn list_memory_files_rejects_non_directory_target() {
    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    tokio::fs::create_dir_all(tmp.path().join("memory"))
        .await
        .expect("create memory root");
    tokio::fs::write(tmp.path().join("memory").join("single.md"), "hello")
        .await
        .expect("write file");

    let err = ai_list_memory_files(ListMemoryFilesRequest {
        relative_dir: "single.md".to_string(),
    })
    .await
    .expect_err("listing a file path should fail");
    assert!(
        err.contains("memory directory not found") || err.contains("resolve memory path"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn read_and_write_memory_files_reject_path_traversal() {
    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let write_err = ai_write_memory_file(WriteMemoryFileRequest {
        relative_path: "../secrets.txt".to_string(),
        content: "nope".to_string(),
    })
    .await
    .expect_err("path traversal should fail for writes");
    assert!(write_err.contains("path traversal is not allowed"));

    let read_err = ai_read_memory_file(ReadMemoryFileRequest {
        relative_path: "../secrets.txt".to_string(),
    })
    .await
    .expect_err("path traversal should fail for reads");
    assert!(read_err.contains("path traversal is not allowed"));
}

#[tokio::test]
async fn list_and_read_memory_files_reject_absolute_paths() {
    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let list_err = ai_list_memory_files(ListMemoryFilesRequest {
        relative_dir: "/tmp".to_string(),
    })
    .await
    .expect_err("absolute list path should fail");
    assert!(list_err.contains("absolute paths are not allowed"));

    let read_err = ai_read_memory_file(ReadMemoryFileRequest {
        relative_path: "/tmp/secret.txt".to_string(),
    })
    .await
    .expect_err("absolute read path should fail");
    assert!(read_err.contains("absolute paths are not allowed"));

    let write_err = ai_write_memory_file(WriteMemoryFileRequest {
        relative_path: "/tmp/secret.txt".to_string(),
        content: "nope".to_string(),
    })
    .await
    .expect_err("absolute write path should fail");
    assert!(write_err.contains("absolute paths are not allowed"));
}

#[tokio::test]
async fn read_memory_file_surfaces_missing_file_error() {
    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let err = ai_read_memory_file(ReadMemoryFileRequest {
        relative_path: "missing.md".to_string(),
    })
    .await
    .expect_err("missing file should fail");
    assert!(
        err.contains("resolve memory path") || err.contains("read memory file"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn read_memory_file_surfaces_invalid_utf8_error() {
    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let memory_root = super::super::helpers::resolve_existing_memory_path("")
        .await
        .expect("resolve memory root");
    tokio::fs::write(memory_root.join("binary.bin"), [0xff, 0xfe, 0xfd])
        .await
        .expect("write invalid utf8 file");

    let err = ai_read_memory_file(ReadMemoryFileRequest {
        relative_path: "binary.bin".to_string(),
    })
    .await
    .expect_err("invalid utf8 file should fail");
    assert!(err.contains("read memory file"));
}

#[cfg(unix)]
#[tokio::test]
async fn write_memory_file_rejects_symlink_targets() {
    use std::os::unix::fs::symlink;

    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let memory_root = super::super::helpers::resolve_existing_memory_path("")
        .await
        .expect("resolve memory root");
    let real = memory_root.join("real.md");
    tokio::fs::write(&real, "hello")
        .await
        .expect("write real file");
    symlink(&real, memory_root.join("alias.md")).expect("create symlink");

    let err = ai_write_memory_file(WriteMemoryFileRequest {
        relative_path: "alias.md".to_string(),
        content: "mutate".to_string(),
    })
    .await
    .expect_err("writing through symlink should fail");
    assert!(err.contains("refusing to write through symlink"));
}

#[cfg(unix)]
#[tokio::test]
async fn list_memory_files_skips_symlink_entries() {
    use std::os::unix::fs::symlink;

    let _guard = TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let memory_root = super::super::helpers::resolve_existing_memory_path("")
        .await
        .expect("resolve memory root");
    let real = memory_root.join("real.md");
    tokio::fs::write(&real, "hello")
        .await
        .expect("write real file");
    symlink(&real, memory_root.join("alias.md")).expect("create symlink");

    let listed = ai_list_memory_files(ListMemoryFilesRequest {
        relative_dir: String::new(),
    })
    .await
    .expect("list should succeed");
    let listed_data = listed.value.data.expect("list data");
    // This test pins the symlink-skipping invariant; SQLite engine files
    // (`memory.db`, `memory.db-shm`, `memory.db-wal`) are exercised by
    // `list_memory_files_skips_internal_sqlite_artifacts` above.
    assert!(
        listed_data.files.iter().any(|f| f == "real.md"),
        "expected real.md in listing, got {:?}",
        listed_data.files
    );
    assert!(
        !listed_data.files.iter().any(|f| f == "alias.md"),
        "symlink alias.md must be skipped, got {:?}",
        listed_data.files
    );
}
