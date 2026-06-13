//! Process-global memory client singleton.
//!
//! One `MemoryClient` (and its background ingestion-queue worker) lives for the
//! entire core process. Every subsystem — RPC handlers, node runtime, screen
//! intelligence, CLI — shares this single instance so the worker is never
//! prematurely dropped.
//!
//! # Usage
//!
//! ```ignore
//! // At startup (core server, CLI, etc.)
//! memory::global::init(workspace_dir)?;
//!
//! // Anywhere that needs to write/read memory:
//! let client = memory::global::client()?;
//! client.put_doc(input).await?;
//! ```

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use crate::openhuman::memory_store::{MemoryClient, MemoryClientRef};

#[derive(Clone)]
struct GlobalMemoryClient {
    workspace_dir: PathBuf,
    client: MemoryClientRef,
}

type GlobalClientSlot = RwLock<Option<GlobalMemoryClient>>;

/// The process-global memory client slot.
static GLOBAL_CLIENT: OnceLock<GlobalClientSlot> = OnceLock::new();

fn global_slot() -> &'static GlobalClientSlot {
    GLOBAL_CLIENT.get_or_init(GlobalClientSlot::default)
}

/// Initialise or re-bind the global memory client from a workspace directory.
///
/// Safe to call multiple times. Calls for the same workspace return the
/// existing client; calls for a different workspace replace the global handle
/// so a post-login active-user switch does not keep writing to the pre-login
/// workspace.
pub fn init(workspace_dir: PathBuf) -> Result<MemoryClientRef, String> {
    init_in_slot(global_slot(), workspace_dir)
}

fn init_in_slot(
    slot: &GlobalClientSlot,
    workspace_dir: PathBuf,
) -> Result<MemoryClientRef, String> {
    if let Some(existing) = slot
        .read()
        .map_err(|e| format!("[memory:global] read lock poisoned: {e}"))?
        .as_ref()
    {
        if existing.workspace_dir == workspace_dir {
            log::debug!("[memory:global] already initialised for current workspace");
            return Ok(Arc::clone(&existing.client));
        }
    }

    log::info!(
        "[memory:global] initialising global MemoryClient workspace={}",
        workspace_dir.display()
    );
    let client = match MemoryClient::from_workspace_dir(workspace_dir.clone()) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            let mut guard = slot
                .write()
                .map_err(|e| format!("[memory:global] write lock poisoned: {e}"))?;
            if guard
                .as_ref()
                .is_some_and(|existing| existing.workspace_dir != workspace_dir)
            {
                log::warn!(
                    "[memory:global] clearing stale MemoryClient after failed rebind to {}",
                    workspace_dir.display()
                );
                *guard = None;
            }
            return Err(error);
        }
    };

    let mut guard = slot
        .write()
        .map_err(|e| format!("[memory:global] write lock poisoned: {e}"))?;
    if let Some(existing) = guard.as_ref() {
        if existing.workspace_dir == workspace_dir {
            return Ok(Arc::clone(&existing.client));
        }

        log::info!(
            "[memory:global] rebinding MemoryClient workspace {} -> {}",
            existing.workspace_dir.display(),
            workspace_dir.display()
        );
    }

    *guard = Some(GlobalMemoryClient {
        workspace_dir,
        client: Arc::clone(&client),
    });
    Ok(client)
}

/// Initialise using the default `~/.openhuman/workspace` directory.
///
/// **TEST-ONLY.** Production code must call [`init`] with the real workspace
/// directory at startup wiring. If this function ran first in production it
/// would pin the singleton to `~/.openhuman/workspace`, causing every
/// subsequent `init(custom_workspace)` to silently no-op and return the wrong
/// handle (`OnceLock::set` is one-shot).
#[cfg(test)]
pub fn init_default() -> Result<MemoryClientRef, String> {
    let workspace_dir = crate::openhuman::config::default_root_openhuman_dir()
        .map_err(|e| e.to_string())?
        .join("workspace");
    init(workspace_dir)
}

/// Returns the global memory client.
///
/// Returns `Err` if [`init`] has not yet been called. There is **no** lazy
/// fallback: a fallback would pin the global to `~/.openhuman/workspace` on
/// the first stray call (test, early RPC, etc.). The explicit init/rebind path
/// keeps workspace ownership visible at startup and after login.
///
/// Callers that can tolerate "not yet ready" should use
/// [`client_if_ready`] instead.
pub fn client() -> Result<MemoryClientRef, String> {
    client_from(global_slot())
}

/// Implementation backing [`client`] — extracted so unit tests can pass a
/// freshly-constructed local slot and assert the uninitialised-error
/// contract without racing the process-global singleton.
fn client_from(slot: &GlobalClientSlot) -> Result<MemoryClientRef, String> {
    slot.read()
        .map_err(|e| format!("[memory:global] read lock poisoned: {e}"))?
        .as_ref()
        .map(|entry| Arc::clone(&entry.client))
        .ok_or_else(|| {
            "memory global accessed before init — call init(workspace) at startup".to_string()
        })
}

/// Returns the global client if already initialised, without lazy init.
pub fn client_if_ready() -> Option<MemoryClientRef> {
    global_slot()
        .read()
        .ok()?
        .as_ref()
        .map(|entry| Arc::clone(&entry.client))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// All tests that touch `GLOBAL_CLIENT` must contend with process-wide
    /// state. We tolerate both branches so test ordering doesn't flake the
    /// suite.
    #[tokio::test]
    async fn client_if_ready_is_some_after_init_or_remains_none() {
        let before = client_if_ready();
        let tmp = TempDir::new().unwrap();
        let _ = init(tmp.path().join("ws"));
        let after = client_if_ready();
        if before.is_some() {
            assert!(after.is_some(), "if global was set, it must remain set");
        } else {
            // First setter wins; if our init succeeded it's set now.
            assert!(after.is_some());
        }
    }

    #[tokio::test]
    async fn init_returns_existing_client_when_already_set() {
        let slot = GlobalClientSlot::default();
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("ws");

        let first = init_in_slot(&slot, workspace.clone()).unwrap();
        let second = init_in_slot(&slot, workspace).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn init_rebinds_client_when_workspace_changes() {
        let slot = GlobalClientSlot::default();
        let tmp = TempDir::new().unwrap();

        let first = init_in_slot(&slot, tmp.path().join("ws-a")).unwrap();
        let second = init_in_slot(&slot, tmp.path().join("ws-b")).unwrap();
        let current = client_from(&slot).unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&second, &current));
    }

    #[tokio::test]
    async fn init_clears_existing_client_when_rebind_workspace_cannot_initialise() {
        let slot = GlobalClientSlot::default();
        let tmp = TempDir::new().unwrap();

        let _first = init_in_slot(&slot, tmp.path().join("ws-a")).unwrap();
        let file_path = tmp.path().join("not-a-directory");
        std::fs::write(&file_path, b"not a workspace").unwrap();

        let err = match init_in_slot(&slot, file_path) {
            Ok(_) => panic!("rebind to a file path must fail"),
            Err(err) => err,
        };

        assert!(err.contains("Create workspace dir"));
        assert!(client_from(&slot).is_err());
    }

    #[tokio::test]
    async fn client_returns_a_handle_after_explicit_init() {
        // Bind TempDir at test scope so its directory outlives the global
        // client — the singleton holds the path and may be used later in
        // this test binary.
        let tmp = TempDir::new().unwrap();
        // Explicit init: client() no longer lazily initialises.
        let _ = client_if_ready().or_else(|| init(tmp.path().join("ws")).ok());
        let c = client().expect("global client should be available after init");
        let _arc: Arc<MemoryClient> = c;
    }

    #[tokio::test]
    async fn client_errs_clearly_when_not_initialised() {
        // Use a fresh local `OnceLock` rather than the process-global one:
        // other tests may have already called `init()` on the singleton, so
        // an `is_none`-gated check on `GLOBAL_CLIENT` would race / silently
        // skip. `client_from` lets us assert the contract deterministically.
        let local = GlobalClientSlot::default();
        match client_from(&local) {
            Ok(_) => panic!("client_from(empty) must error"),
            Err(err) => assert!(
                err.contains("init"),
                "error should mention init contract, got: {err}"
            ),
        }
    }
}
