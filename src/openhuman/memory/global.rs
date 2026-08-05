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

use crate::openhuman::memory::store::{MemoryClient, MemoryClientRef};

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

/// Rebuild the global client for the workspace it is already bound to.
///
/// [`init`] deliberately no-ops when the workspace is unchanged, which is right
/// for its callers — but wrong for a change in *how* memory embeds rather than
/// *where* it lives. `UnifiedMemory` resolves its embedder once, at
/// construction, so after an embedding provider / model / dimension switch the
/// live client still holds the old one: the base re-embed sweep would then scan
/// and write under the superseded signature, leaving every row that the switch
/// just made pending stuck that way until the next boot.
///
/// Returns `Ok(None)` when nothing is bound yet — there is no client to
/// replace, and the next [`init`] picks up the saved config anyway. A rebuild
/// failure leaves the existing client in place: a stale embedder still answers
/// queries, where an empty slot answers nothing.
pub fn rebind_current_workspace() -> Result<Option<MemoryClientRef>, String> {
    rebind_in_slot(global_slot())
}

fn rebind_in_slot(slot: &GlobalClientSlot) -> Result<Option<MemoryClientRef>, String> {
    log::debug!("[memory:global] rebind entry");
    let workspace_dir = {
        let guard = slot
            .read()
            .map_err(|e| format!("[memory:global] read lock poisoned: {e}"))?;
        match guard.as_ref() {
            Some(existing) => existing.workspace_dir.clone(),
            None => {
                log::debug!("[memory:global] rebind exit branch=nothing-bound");
                return Ok(None);
            }
        }
    };
    rebind_in_slot_for(slot, workspace_dir)
}

/// The build-and-install half of [`rebind_in_slot`], split out so the
/// install-time recheck can be exercised directly: a test can hand it a
/// workspace the slot has since moved off, which is what the released lock
/// makes possible in production.
fn rebind_in_slot_for(
    slot: &GlobalClientSlot,
    workspace_dir: PathBuf,
) -> Result<Option<MemoryClientRef>, String> {
    log::info!(
        "[memory:global] rebuilding MemoryClient in place workspace={}",
        workspace_dir.display()
    );
    let client = match MemoryClient::from_workspace_dir(workspace_dir.clone()) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            log::debug!(
                "[memory:global] rebind exit branch=build-failed workspace={} error={error}",
                workspace_dir.display()
            );
            return Err(error);
        }
    };

    let mut guard = slot
        .write()
        .map_err(|e| format!("[memory:global] write lock poisoned: {e}"))?;
    // The lock is released while the client is built, so an `init` for a
    // different workspace (a post-login active-user switch) can land in the
    // gap. Installing unconditionally would then overwrite that newer binding
    // with a client for the workspace this rebind started from, and every write
    // after it would go to the previous user's store. A rebind is only ever
    // valid for the binding it read.
    match guard.as_ref() {
        Some(current) if current.workspace_dir == workspace_dir => {}
        _ => {
            log::info!(
                "[memory:global] discarding rebuilt MemoryClient for {} — the global rebound \
                 while it was being built",
                workspace_dir.display()
            );
            return Ok(None);
        }
    }
    log::debug!(
        "[memory:global] rebind exit branch=installed workspace={}",
        workspace_dir.display()
    );
    *guard = Some(GlobalMemoryClient {
        workspace_dir,
        client: Arc::clone(&client),
    });
    Ok(Some(client))
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

    /// `init` returning the same handle for an unchanged workspace is what makes
    /// a dedicated rebind necessary: an embedder swap changes how memory embeds,
    /// not where it lives, so the workspace-keyed short-circuit would keep the
    /// old embedder alive and the base sweep would run under a dead signature.
    #[tokio::test]
    async fn rebind_replaces_the_client_for_the_same_workspace() {
        let slot = GlobalClientSlot::default();
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("ws");

        let first = init_in_slot(&slot, workspace.clone()).unwrap();
        // The path `init` takes for an unchanged workspace, spelled out.
        assert!(Arc::ptr_eq(
            &first,
            &init_in_slot(&slot, workspace.clone()).unwrap()
        ));

        let rebound = rebind_in_slot(&slot).unwrap().expect("a client was bound");
        let current = client_from(&slot).unwrap();

        assert!(
            !Arc::ptr_eq(&first, &rebound),
            "rebind must build a new client, not hand back the bound one"
        );
        assert!(Arc::ptr_eq(&rebound, &current), "and must install it");
    }

    /// The window the rebind opens: it releases the read lock to build, so an
    /// `init` for a different workspace can land before it installs. Installing
    /// anyway would point the global at the workspace the rebind *started*
    /// from — the pre-login store, after a post-login user switch. Simulated by
    /// rebinding the slot out from under a client built for the old workspace.
    #[tokio::test]
    async fn rebind_does_not_overwrite_a_workspace_bound_while_it_was_building() {
        let slot = GlobalClientSlot::default();
        let tmp = TempDir::new().unwrap();
        let workspace_a = tmp.path().join("ws-a");

        let _a = init_in_slot(&slot, workspace_a.clone()).unwrap();
        // Stand in for the build window: the switch happens before the install.
        let b = init_in_slot(&slot, tmp.path().join("ws-b")).unwrap();

        // A rebind that read `ws-a` must find the slot changed and stand down.
        let stale = rebind_in_slot_for(&slot, workspace_a).unwrap();

        assert!(stale.is_none(), "a stale rebind must not install");
        assert!(
            Arc::ptr_eq(&client_from(&slot).unwrap(), &b),
            "the newer binding must survive"
        );
    }

    #[tokio::test]
    async fn rebind_is_a_no_op_when_nothing_is_bound() {
        let slot = GlobalClientSlot::default();
        assert!(rebind_in_slot(&slot).unwrap().is_none());
        assert!(client_from(&slot).is_err());
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
