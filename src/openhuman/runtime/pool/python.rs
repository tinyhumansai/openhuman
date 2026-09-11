//! Python pooled execution: whether this host wants it, and how to ask for it.

use std::path::PathBuf;
use std::time::Duration;

use tinyruntime_bus::Language;

use super::{PoolExecOutcome, PoolRunError};
use crate::openhuman::config::{Config, RuntimePoolConfig};

/// Whether inline `python` jobs should route through the pool.
///
/// Python defaults **off**, and the asymmetry with Node is real rather than an
/// oversight. Jobs share one interpreter — CPython offers no worker-thread
/// equivalent and no safe way to kill a running thread — so reuse leaks
/// process-global state (`sys.modules`, `os.environ`, logging handlers, threads)
/// across otherwise unrelated runs. Opt in explicitly
/// (`[runtime_pool.python] enabled = true`) to accept that in exchange for the
/// warm-worker memory saving.
#[must_use]
pub fn enabled(pool: &RuntimePoolConfig) -> bool {
    pool.enabled && pool.python.is_enabled(false)
}

/// Run inline Python on a pooled, warm `python` worker.
///
/// # Errors
///
/// [`PoolRunError`], classified so the caller knows whether falling back to a
/// per-call spawn is safe.
pub async fn run_inline(
    config: &Config,
    code: String,
    cwd: Option<PathBuf>,
    timeout: Option<Duration>,
) -> Result<PoolExecOutcome, PoolRunError> {
    super::run_inline(config, &Language::python(), code, cwd, timeout).await
}
