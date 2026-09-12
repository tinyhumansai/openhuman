use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;

use super::{build_backend_at, KeyringBackend};

thread_local! {
    /// Workspace bound by [`ScopedWorkspace`] on this thread, if any.
    static SCOPED_WORKSPACE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Fallback workspace for tests that do not bind one: stable for the whole
/// process, outside the developer's home, and never deleted mid-run.
static PROCESS_DEFAULT: OnceLock<PathBuf> = OnceLock::new();

/// One backend per resolved workspace directory.
#[allow(clippy::type_complexity)]
static BACKENDS: OnceLock<Mutex<HashMap<PathBuf, &'static dyn KeyringBackend>>> = OnceLock::new();

/// The keyring workspace for the calling thread.
pub(crate) fn current_workspace() -> PathBuf {
    if let Some(dir) = SCOPED_WORKSPACE.with(|cell| cell.borrow().clone()) {
        return dir;
    }
    PROCESS_DEFAULT
        .get_or_init(|| {
            std::env::temp_dir().join(format!("openhuman-keyring-tests-{}", std::process::id()))
        })
        .clone()
}

/// The backend rooted at `dir`, built once per directory.
pub(crate) fn backend_for(dir: &Path) -> &'static dyn KeyringBackend {
    let registry = BACKENDS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(existing) = registry.lock().get(dir).copied() {
        return existing;
    }
    // Built outside the registry lock: backend construction can re-enter the
    // keyring module, and holding the lock across it would deadlock.
    let candidate: &'static dyn KeyringBackend = Box::leak(build_backend_at(dir));
    *registry
        .lock()
        .entry(dir.to_path_buf())
        .or_insert(candidate)
}

/// Binds the calling thread's keyring workspace for the guard's lifetime.
///
/// Thread-scoped rather than process-scoped, so a test that needs a private
/// credential store cannot redirect the secrets of tests running in
/// parallel — which is exactly what the `OPENHUMAN_WORKSPACE` env guards
/// used to do.
pub(crate) struct ScopedWorkspace {
    previous: Option<PathBuf>,
}

impl ScopedWorkspace {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        let previous = SCOPED_WORKSPACE.with(|cell| cell.borrow_mut().replace(dir.into()));
        Self { previous }
    }
}

impl Drop for ScopedWorkspace {
    fn drop(&mut self) {
        let previous = self.previous.take();
        SCOPED_WORKSPACE.with(|cell| *cell.borrow_mut() = previous);
    }
}
