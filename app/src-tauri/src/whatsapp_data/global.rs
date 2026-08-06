//! Process-global WhatsApp data store singleton.
//!
//! One workspace-bound `WhatsAppDataStore` is active at a time, shared by
//! native handlers, scanners, and Tauri commands. When the active workspace
//! changes without relaunching the shell, the singleton is reopened for the new
//! path so user data cannot leak across sessions.
//!
//! # Usage
//!
//! ```ignore
//! // At startup:
//! whatsapp_data::global::init(workspace_dir)?;
//!
//! // In RPC handlers:
//! let store = whatsapp_data::global::store()?;
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use super::store::WhatsAppDataStore;

/// Shared, thread-safe reference to the store.
pub type WhatsAppDataStoreRef = Arc<WhatsAppDataStore>;

// `RwLock<Option<…>>` rather than `OnceLock` so tests can swap workspaces
// between runs (each test uses its own temp dir; without reset, the second
// test would attach to a dropped sqlite path). Production callers still get
// strict idempotency: `init` is a no-op once a store is set.
struct WorkspaceStore {
    workspace_dir: PathBuf,
    store: WhatsAppDataStoreRef,
}

static GLOBAL_STORE: RwLock<Option<WorkspaceStore>> = RwLock::new(None);

/// Initialise the global store for `workspace_dir`.
///
/// Reuses the current instance only when it is bound to the same workspace.
/// A different path atomically replaces it with a freshly opened store.
pub fn init(workspace_dir: PathBuf) -> Result<WhatsAppDataStoreRef, String> {
    let mut guard = GLOBAL_STORE
        .write()
        .map_err(|e| format!("[whatsapp_data:global] write lock poisoned: {e}"))?;
    if let Some(existing) = guard
        .as_ref()
        .filter(|entry| same_workspace(&entry.workspace_dir, &workspace_dir))
    {
        log::debug!("[whatsapp_data:global] already initialised");
        return Ok(Arc::clone(&existing.store));
    }
    log::info!(
        "[whatsapp_data:global] opening store workspace={}",
        workspace_dir.display()
    );
    let store = Arc::new(
        WhatsAppDataStore::new(&workspace_dir)
            .map_err(|e| format!("[whatsapp_data] store init failed: {e}"))?,
    );
    *guard = Some(WorkspaceStore {
        workspace_dir,
        store: Arc::clone(&store),
    });
    Ok(store)
}

fn same_workspace(current: &Path, requested: &Path) -> bool {
    current == requested
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_reuses_same_workspace_and_reopens_when_workspace_changes() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();

        let first = init(first_dir.path().to_path_buf()).unwrap();
        let same = init(first_dir.path().to_path_buf()).unwrap();
        assert!(Arc::ptr_eq(&first, &same));

        let second = init(second_dir.path().to_path_buf()).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(second_dir
            .path()
            .join("whatsapp_data/whatsapp_data.db")
            .exists());
    }
}
