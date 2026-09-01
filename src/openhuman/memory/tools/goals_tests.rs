//! Tests for the four agent-facing goals tools.
//!
//! These used to sandbox each tool with its own `tempfile::tempdir()`, because
//! the tool held the workspace path and opened `MEMORY_GOALS.md` under it. The
//! store is behind the loaded module now and the document is reached through
//! the ambient binding, so isolation moves to where the binding reads it from:
//! `OPENHUMAN_WORKSPACE`, pinned for the test and restored after — the same
//! `isolated_config` shape `tools/tool_memory/put_tests.rs` uses for the same
//! reason. `GLOBAL_MEMORY_TEST_LOCK` serialises them, since the workspace env
//! var is process-wide.

use super::*;

use std::ffi::OsString;

use tempfile::TempDir;

use crate::openhuman::config::TEST_ENV_LOCK;
use crate::openhuman::tools::traits::Tool;

struct WorkspaceEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", path);
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var("OPENHUMAN_WORKSPACE", previous);
        } else {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}

/// Reset the shared goals document, and say why that has to happen at all.
///
/// `OPENHUMAN_WORKSPACE` does NOT isolate these tests: the test module
/// provider deliberately pins every binding to
/// `ops::shared_memory_test_workspace()` (see `memory/binding.rs`'s test
/// `module_provider` — a native module is loaded once per process and captures
/// its first workspace), so goals written here land in the SHARED subtree and
/// outlive the tempdir. Each test therefore clears the document itself, while
/// it holds `GLOBAL_MEMORY_TEST_LOCK`, instead of trusting the env guard.
async fn reset_shared_goals() {
    let guard = crate::openhuman::memory::ops::guard::active_memory_guard()
        .await
        .expect("resolve the shared test memory guard");
    if let Some(goals) = guard.as_goals() {
        goals
            .set_goals(crate::openhuman::memory::api::goals::GoalsDoc::default())
            .await
            .expect("reset the shared goals document");
    }
}

#[tokio::test]
async fn add_then_list_reflects_change() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    reset_shared_goals().await;

    let add = GoalsAddTool::new(tmp.path().to_path_buf());
    let res = add
        .execute(json!({ "text": "help ship the app" }))
        .await
        .unwrap();
    assert!(!res.is_error, "add failed: {}", res.text());

    let list = GoalsListTool::new(tmp.path().to_path_buf());
    let res = list.execute(json!({})).await.unwrap();
    assert!(res.text().contains("help ship the app"));
}

#[tokio::test]
async fn edit_and_delete_unknown_id_error() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    reset_shared_goals().await;

    let edit = GoalsEditTool::new(tmp.path().to_path_buf());
    let res = edit
        .execute(json!({ "id": "g9", "text": "x" }))
        .await
        .unwrap();
    assert!(res.is_error);

    let del = GoalsDeleteTool::new(tmp.path().to_path_buf());
    let res = del.execute(json!({ "id": "g9" })).await.unwrap();
    assert!(res.is_error);
}

/// The host-side guards reach the agent through the tool result, not only
/// through the RPC surface — the `goals_agent` is the main caller and this is
/// the text it retries against.
#[tokio::test]
async fn add_refuses_pii_bearing_text_with_the_specific_reason() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    reset_shared_goals().await;

    let add = GoalsAddTool::new(tmp.path().to_path_buf());
    let res = add
        .execute(json!({ "text": "follow up with alice@example.com" }))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(
        res.text().contains("secrets or PII"),
        "unexpected refusal text: {}",
        res.text()
    );
}

/// Argument validation is the tool's own and runs before anything is resolved,
/// so it holds whatever the driver is.
#[tokio::test]
async fn missing_arguments_are_reported_as_tool_errors() {
    let dir = std::env::temp_dir();
    assert!(
        GoalsAddTool::new(dir.clone())
            .execute(json!({}))
            .await
            .unwrap()
            .is_error
    );
    assert!(
        GoalsEditTool::new(dir.clone())
            .execute(json!({ "id": "g1" }))
            .await
            .unwrap()
            .is_error
    );
    assert!(
        GoalsDeleteTool::new(dir)
            .execute(json!({}))
            .await
            .unwrap()
            .is_error
    );
}
