//! Shared test infrastructure for `memory::ops` submodule tests.
//!
//! All `ops` submodules that need a global [`MemoryClient`] call
//! [`ensure_shared_memory_client`] instead of creating their own
//! `OnceLock<PathBuf>`.  Sharing one leaked workspace means concurrent
//! `global::init()` calls always resolve to the same path and hit the
//! no-op fast-path inside `init_in_slot`, preventing one test thread
//! from silently rebinding the global under another thread's feet.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Binds the process-global memory client to a single shared temp workspace and
/// returns that workspace path.
///
/// Safe to call from multiple test threads concurrently — subsequent calls with
/// the same workspace path return the existing client without rebinding.
///
/// The returned path lets callers whose RPC path *also* resolves the workspace
/// from `OPENHUMAN_WORKSPACE` (notably `memory::ops::documents` via
/// `memory_init` → `current_workspace_dir`) pin the env var to this same path so
/// the env and the bound client agree. See `documents::tests`.
pub(crate) fn ensure_shared_memory_client() -> PathBuf {
    static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
    let workspace = WORKSPACE.get_or_init(|| {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("workspace");
        std::fs::create_dir_all(&path).expect("workspace dir");
        std::mem::forget(tmp);
        path
    });
    crate::openhuman::memory::global::init(workspace.clone())
        .expect("initialize shared test memory client");
    workspace.clone()
}
