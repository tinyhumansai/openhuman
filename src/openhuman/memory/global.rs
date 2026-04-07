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

    // Pre-warm the GLiNER model in the background so first ingestion is fast.
    tokio::spawn(async {
        let _ = super::relex::warm_default_bundle().await;
    });

    Ok(GLOBAL_CLIENT.get().cloned().unwrap_or(client))
}

/// Initialise using the default `.openhuman/workspace` directory.
pub fn init_default() -> Result<MemoryClientRef, String> {
    let workspace_dir = dirs::home_dir()
        .ok_or_else(|| "failed to resolve home directory".to_string())?
        .join(".openhuman")
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
