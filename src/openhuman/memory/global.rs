//! Process-global memory client singleton.
//!
//! One `MemoryClient` (and its background ingestion-queue worker) lives for the
//! entire core process. Every subsystem — RPC handlers, skills runtime, screen
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
use std::sync::{Arc, OnceLock};

use crate::openhuman::memory::{MemoryClient, MemoryClientRef};

/// The process-global memory client.
static GLOBAL_CLIENT: OnceLock<MemoryClientRef> = OnceLock::new();

/// Initialise the global memory client from a workspace directory.
///
/// Safe to call multiple times — only the first call takes effect.
/// Returns the (possibly pre-existing) client reference.
pub fn init(workspace_dir: PathBuf) -> Result<MemoryClientRef, String> {
    if let Some(existing) = GLOBAL_CLIENT.get() {
        log::debug!("[memory:global] already initialised, returning existing client");
        return Ok(Arc::clone(existing));
    }

    log::info!(
        "[memory:global] initialising global MemoryClient workspace={}",
        workspace_dir.display()
    );
    let client = Arc::new(MemoryClient::from_workspace_dir(workspace_dir)?);

    // OnceLock::set can fail if another thread raced us — that's fine,
    // just return whichever won.
    let _ = GLOBAL_CLIENT.set(Arc::clone(&client));

    Ok(GLOBAL_CLIENT.get().cloned().unwrap_or(client))
}

/// Initialise using the default `.openhuman/workspace` directory.
pub fn init_default() -> Result<MemoryClientRef, String> {
    let workspace_dir = crate::openhuman::config::default_root_openhuman_dir()
        .map_err(|e| e.to_string())?
        .join("workspace");
    init(workspace_dir)
}

/// Returns the global memory client, lazily initialising with default paths
/// if not yet set up.
///
/// Prefer calling `init()` explicitly at startup so errors surface early.
pub fn client() -> Result<MemoryClientRef, String> {
    if let Some(c) = GLOBAL_CLIENT.get() {
        return Ok(Arc::clone(c));
    }
    // Lazy fallback — initialise with defaults.
    init_default()
}

/// Returns the global client if already initialised, without lazy init.
pub fn client_if_ready() -> Option<MemoryClientRef> {
    GLOBAL_CLIENT.get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// All tests must contend with the fact that `GLOBAL_CLIENT` is a
    /// process-wide `OnceLock` — once set, it stays set for the rest of
    /// the test binary. We tolerate both branches so test ordering doesn't
    /// flake the suite.
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
        let tmp = TempDir::new().unwrap();
        let first = init(tmp.path().join("ws-a"));
        let tmp2 = TempDir::new().unwrap();
        let second = init(tmp2.path().join("ws-b"));
        assert!(first.is_ok() && second.is_ok());
        // Both refs point to the same global Arc — the second init is a no-op.
        assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
    }

    #[tokio::test]
    async fn client_returns_a_handle_either_via_lazy_init_or_existing() {
        // Bind TempDir at test scope so its directory outlives any lazy
        // init — the global client holds the path and can be used later in
        // this test (and potentially by other tests in the same binary).
        let tmp = TempDir::new().unwrap();
        let _ = client_if_ready().or_else(|| init(tmp.path().join("ws")).ok());
        let c = client().expect("global client should be available");
        let _arc: Arc<MemoryClient> = c;
    }
}
