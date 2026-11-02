//! The workspace default for code that runs with no backend installed.
//!
//! Hosts install a backend at startup ([`super::install_for_url`]) and every
//! store reaches it through [`super::installed`]. An embedder that never ran
//! the host boot installs none; such a call opens **the same default a host
//! would**: `sqlite:<workspace>/storage` (v1) or
//! `sqlite:<user dir>/storage.db` (v2) ([`super::config::default_url`]), once
//! per workspace for the life of the process. A different workspace is a
//! different database. Under `cfg(test)`, ordinary unit tests use the TSD
//! `MemoryBackend` testkit instead; tests of durable reopen behavior explicitly
//! opt into the SQLite default.
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

use super::{current_scope, installed, Scope, Storage, StorageBackend};

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
    pub fn scoped(&self) -> Result<Storage> {
        self.backend
            .for_scope(&self.scope)
            .map_err(|error| anyhow!("open the storage scope: {error}"))
    }
}

/// Default backends this process opened for workspaces with nothing
/// installed, by workspace directory.
type WorkspaceBackend = Arc<tokio::sync::OnceCell<Arc<dyn StorageBackend>>>;

static WORKSPACES: LazyLock<Mutex<HashMap<PathBuf, WorkspaceBackend>>> =
    LazyLock::new(Mutex::default);

#[cfg(test)]
static TEST_WORKSPACE_URLS: LazyLock<Mutex<HashMap<PathBuf, String>>> =
    LazyLock::new(Mutex::default);

#[cfg(test)]
pub(crate) fn use_persistent_default_for_test(workspace_dir: &Path) {
    TEST_WORKSPACE_URLS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            workspace_dir.to_path_buf(),
            super::config::default_url(workspace_dir),
        );
}

#[cfg(test)]
struct BackendOpenTestGate {
    started: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[cfg(test)]
static BACKEND_OPEN_TEST_GATE: LazyLock<Mutex<Option<BackendOpenTestGate>>> =
    LazyLock::new(Mutex::default);

#[cfg(test)]
struct SyncBackendEntryTestSignal {
    workspace_dir: PathBuf,
    sender: std::sync::mpsc::Sender<()>,
}

#[cfg(test)]
static SYNC_BACKEND_ENTRY_TEST_SIGNAL: LazyLock<Mutex<Option<SyncBackendEntryTestSignal>>> =
    LazyLock::new(Mutex::default);

#[cfg(test)]
pub(super) fn install_test_backend_open_gate(
    started: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
) {
    *BACKEND_OPEN_TEST_GATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(BackendOpenTestGate { started, release });
}

#[cfg(test)]
pub(super) fn arm_test_sync_backend_entry_signal(
    workspace_dir: &Path,
    sender: std::sync::mpsc::Sender<()>,
) {
    *SYNC_BACKEND_ENTRY_TEST_SIGNAL
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(SyncBackendEntryTestSignal {
        workspace_dir: workspace_dir.to_path_buf(),
        sender,
    });
}

#[cfg(test)]
pub(super) fn clear_test_sync_backend_entry_signal(workspace_dir: &Path) {
    let mut signal = SYNC_BACKEND_ENTRY_TEST_SIGNAL
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if signal
        .as_ref()
        .is_some_and(|signal| signal.workspace_dir == workspace_dir)
    {
        signal.take();
    }
}

#[cfg(test)]
async fn await_test_backend_open_gate() {
    let gate = BACKEND_OPEN_TEST_GATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(gate) = gate {
        let _ = gate.started.send(());
        let _ = gate.release.await;
    }
}

#[cfg(test)]
fn signal_test_sync_backend_entry(workspace_dir: &Path) {
    let mut signal = SYNC_BACKEND_ENTRY_TEST_SIGNAL
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if signal
        .as_ref()
        .is_some_and(|signal| signal.workspace_dir == workspace_dir)
    {
        if let Some(signal) = signal.take() {
            let _ = signal.sender.send(());
        }
    }
}

fn workspace_backend(workspace_dir: &Path) -> WorkspaceBackend {
    let mut opened = WORKSPACES.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(
        opened
            .entry(workspace_dir.to_path_buf())
            .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new())),
    )
}

async fn open_default_backend(_workspace_dir: PathBuf) -> Result<Arc<dyn StorageBackend>> {
    #[cfg(test)]
    await_test_backend_open_gate().await;

    #[cfg(test)]
    {
        let persistent_url = TEST_WORKSPACE_URLS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&_workspace_dir)
            .cloned();
        if let Some(url) = persistent_url {
            super::config::prepare_default_dir(&_workspace_dir).with_context(|| {
                format!(
                    "create the storage directory for {}",
                    _workspace_dir.display()
                )
            })?;
            return super::open(&url).await.with_context(|| {
                format!("open the default storage for {}", _workspace_dir.display())
            });
        }
    }

    #[cfg(test)]
    {
        let backend: Arc<dyn StorageBackend> =
            Arc::new(super::test_support::UnfencedMemoryBackend::new());
        super::boot::prepare_and_migrate(&backend).await?;
        Ok(super::fenced_backend::wrap(backend))
    }

    #[cfg(not(test))]
    {
        let url = super::config::default_url(&_workspace_dir);
        super::config::prepare_default_dir(&_workspace_dir).with_context(|| {
            format!(
                "create the storage directory for {}",
                _workspace_dir.display()
            )
        })?;
        tracing::debug!(
            target: "openhuman::storage",
            workspace = %_workspace_dir.display(),
            "[storage::local] opening the workspace default backend (none installed)"
        );
        super::open(&url)
            .await
            .with_context(|| format!("open the default storage for {}", _workspace_dir.display()))
    }
}

async fn initialized_backend(
    cell: WorkspaceBackend,
    workspace_dir: PathBuf,
) -> Result<Arc<dyn StorageBackend>> {
    cell.get_or_try_init(|| async move { open_default_backend(workspace_dir).await })
        .await
        .map(Arc::clone)
}

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
    workspace_default(workspace_dir)
}

/// The workspace's own default backend, opened on first use, for a caller that
/// already found no installed backend.
fn workspace_default(workspace_dir: &Path) -> Result<Arc<dyn StorageBackend>> {
    let cell = workspace_backend(workspace_dir);
    if let Some(backend) = cell.get() {
        return Ok(Arc::clone(backend));
    }
    #[cfg(test)]
    signal_test_sync_backend_entry(workspace_dir);
    let workspace_dir = workspace_dir.to_path_buf();
    super::block_on_anyhow(async move { initialized_backend(cell, workspace_dir).await })
}

/// Async counterpart to [`backend_for`] for boot and module-proxy paths that
/// already run inside a future. A cached workspace backend is returned without
/// crossing the synchronous bridge. Cold initialization is delegated to
/// Tokio's blocking pool so the shared OnceCell initializer is driven by the
/// storage bridge rather than by the caller's runtime. When already on the
/// bridge's runtime, initialize directly: queueing a synchronous bridge call
/// from a bridge worker would wait on itself. A synchronous store may wait for
/// the same initializer, including from a current-thread runtime, without
/// preventing it from making progress.
pub async fn backend_for_async(workspace_dir: &Path) -> Result<Arc<dyn StorageBackend>> {
    if let Some(backend) = installed() {
        return Ok(backend);
    }
    let cell = workspace_backend(workspace_dir);
    if let Some(backend) = cell.get() {
        return Ok(Arc::clone(backend));
    }
    let workspace_dir = workspace_dir.to_path_buf();
    if super::ON_STORAGE_RUNTIME.with(std::cell::Cell::get) {
        return initialized_backend(cell, workspace_dir).await;
    }
    // This call already found no installed backend; the worker opens the
    // workspace default rather than asking again from another thread.
    tokio::task::spawn_blocking(move || workspace_default(&workspace_dir))
        .await
        .context("join the workspace storage opener")?
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
