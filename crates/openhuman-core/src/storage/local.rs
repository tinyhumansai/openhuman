//! The workspace default for code that runs with no backend installed.
//!
//! Hosts install a backend at startup ([`super::install_for_url`]) and every
//! store reaches it through [`super::installed`]. A unit test, or an embedder
//! that never ran the host boot, installs none; rather than keep a second,
//! file-shaped implementation of each store for them, such a call opens **the
//! same default a host would**: `sqlite:<workspace>/storage` (v1) or `sqlite:<user dir>/storage.db` (v2)
//! ([`super::config::default_url`]), once per workspace for the life of the
//! process. A different workspace is a different database, which is also what
//! isolates tests that use a temp directory each.
//!
//! A SaaS core with no storage URL lands here too: each profile's workspace is
//! its own database, so nothing is shared between tenants.
//!
//! The fallback imports nothing. The legacy per-domain files are read by
//! [`super::import`] when a backend is installed at boot (and again when the
//! workspace is rebound, [`super::rebind`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use anyhow::{anyhow, Context, Result};

use super::{current_scope, installed, Scope, ScopedStorage, StorageBackend};

/// A backend and the scope a store's records live under in it.
#[derive(Clone)]
pub struct Opened {
    /// The storage backend.
    pub backend: Arc<dyn StorageBackend>,
    /// The scope within it.
    pub scope: Scope,
}

impl Opened {
    /// The backend bound to the scope.
    ///
    /// # Errors
    ///
    /// When the backend refuses the scope.
    pub fn scoped(&self) -> Result<ScopedStorage> {
        self.backend
            .for_scope(&self.scope)
            .map_err(|error| anyhow!("open the storage scope: {error}"))
    }
}

/// Default backends this process opened for workspaces with nothing
/// installed, by workspace directory.
static WORKSPACES: LazyLock<Mutex<HashMap<PathBuf, Arc<dyn StorageBackend>>>> =
    LazyLock::new(Mutex::default);

/// The backend for `workspace_dir`: the installed one, else the workspace's
/// own default, opened on first use.
///
/// # Errors
///
/// In SaaS mode with no backend installed, or when the default cannot be
/// opened.
pub fn backend_for(workspace_dir: &Path) -> Result<Arc<dyn StorageBackend>> {
    if let Some(backend) = installed() {
        return Ok(backend);
    }
    let mut opened = WORKSPACES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(backend) = opened.get(workspace_dir) {
        return Ok(Arc::clone(backend));
    }
    let url = super::config::default_url(workspace_dir);
    super::config::prepare_default_dir(workspace_dir).with_context(|| {
        format!(
            "create the storage directory for {}",
            workspace_dir.display()
        )
    })?;
    tracing::debug!(
        target: "openhuman::storage",
        workspace = %workspace_dir.display(),
        "[storage::local] opening the workspace default backend (none installed)"
    );
    let backend = super::block_on_anyhow({
        let url = url.clone();
        async move { Ok(super::open(&url).await?) }
    })
    .with_context(|| format!("open the default storage for {}", workspace_dir.display()))?;
    opened.insert(workspace_dir.to_path_buf(), Arc::clone(&backend));
    Ok(backend)
}

/// The storage a store uses for this call: [`backend_for`] `workspace_dir`
/// under the acting scope ([`current_scope`]).
///
/// # Errors
///
/// When the backend cannot be had ([`backend_for`]) or the scope cannot be
/// resolved (SaaS mode with no acting profile).
pub fn open(domain: &'static str, workspace_dir: &Path) -> Result<Opened> {
    // The scope first: a call with no acting scope (SaaS mode, no agent) fails
    // closed without creating a workspace database as a side effect.
    let scope = current_scope().with_context(|| format!("[{domain}] resolve the storage scope"))?;
    let backend = backend_for(workspace_dir)
        .with_context(|| format!("[{domain}] open the storage backend"))?;
    Ok(Opened { backend, scope })
}

/// Drops every cached workspace default, so a data reset can delete the files
/// (Windows refuses while a handle is open) and the next call opens afresh.
pub fn release_all() {
    WORKSPACES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

/// Forgets the default opened for `workspace_dir`, so the next call opens it
/// afresh; also how a test models a restart.
pub fn forget(workspace_dir: &Path) {
    WORKSPACES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(workspace_dir);
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;
