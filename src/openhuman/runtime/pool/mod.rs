//! Pooled execution of inline code, delegated to the `tinyruntime` module.
//!
//! The pool itself — warm interpreter children, the newline-delimited job
//! protocol, backpressure, idle reaping, recycle-after-N — moved into that
//! module, where one implementation serves every language. What is left here is
//! the client: whether this host wants pooling for a language, and the mapping
//! from a module reply back onto the shapes the exec tools already handle.
//!
//! # Why the local types survived
//!
//! [`PoolExecOutcome`] and [`PoolRunError`] are what `node_exec` and
//! `python_exec` match on to decide between reporting a result, falling back to
//! a per-call spawn, and refusing to retry. Keeping them meant the migration did
//! not have to rewrite either tool's dispatch logic — and the three-way
//! distinction they encode is the one that keeps a job from running twice.
//!
//! # The fallback is still real
//!
//! A pre-dispatch failure sends the caller to its legacy per-call spawn, which
//! still exists and still works: the tools hold a resolved interpreter path from
//! the module and can run it directly. Pooling is an optimisation seam, exactly
//! as it was — `runtime_pool.enabled = false` reverts every caller with no
//! behavioural change.

pub mod node;
pub mod python;
pub mod types;

use tinyruntime_bus::Language;

use crate::openhuman::config::Config;
use crate::openhuman::runtime::client::{self as runtime, RuntimeCallError};

pub use types::{PoolExecOutcome, PoolLang, PoolSettings};

/// Why a pooled run failed, classified so callers know what is safe to do next.
#[derive(Debug)]
pub enum PoolRunError {
    /// The pool was at capacity and shed the job rather than buffering it.
    ///
    /// Callers must **not** fall back to a per-call spawn: that reintroduces the
    /// very resident memory the pool exists to cap. Surface a busy error or
    /// retry later.
    Saturated,
    /// The job never reached a worker, so it never ran. A retry or a legacy
    /// per-call spawn is safe.
    PreDispatch(anyhow::Error),
    /// The job reached a worker and may have executed. Terminal — re-running it
    /// could duplicate whatever it already did.
    PostDispatch(anyhow::Error),
}

impl std::fmt::Display for PoolRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Saturated => write!(f, "runtime pool at capacity"),
            Self::PreDispatch(error) => write!(f, "pre-dispatch pool failure: {error:#}"),
            Self::PostDispatch(error) => write!(f, "post-dispatch pool failure: {error:#}"),
        }
    }
}

/// Run one inline job on `language`'s pool.
///
/// Shared by the two language backends, which differ only in which language they
/// name — every other decision belongs to the module.
async fn run_inline(
    config: &Config,
    language: &Language,
    code: String,
    cwd: Option<std::path::PathBuf>,
    timeout: Option<std::time::Duration>,
) -> Result<PoolExecOutcome, PoolRunError> {
    let cwd = cwd.map(|path| path.to_string_lossy().into_owned());

    match runtime::execute(config, language, code, cwd, timeout).await {
        Ok(response) => Ok(PoolExecOutcome::from_module(&response)),
        Err(error) => Err(classify(&error)),
    }
}

/// Map a module failure onto the three-way distinction callers act on.
///
/// The saturation and post-dispatch readings come from the module's own error
/// text, which is its contract with a host that renders it. Anything else is
/// pre-dispatch: the conservative reading for a *retryable* classification would
/// be wrong in the other direction — assuming a job ran when it did not merely
/// costs a fallback spawn, while assuming it did not when it did would run
/// someone's code twice.
fn classify(error: &RuntimeCallError) -> PoolRunError {
    let message = error.to_string();
    if message.contains("pool is at capacity") {
        return PoolRunError::Saturated;
    }
    if message.contains("failed after dispatch") {
        return PoolRunError::PostDispatch(anyhow::anyhow!("{message}"));
    }
    PoolRunError::PreDispatch(anyhow::anyhow!("{message}"))
}

/// Every live pool's counters, as the module reports them.
///
/// Returns an empty list when the module is not loaded or has no pool yet, which
/// is the same thing a status surface wants to render: nothing running.
pub async fn all_stats(config: &Config) -> Vec<(PoolLang, tinyruntime_bus::PoolStats)> {
    match runtime::pool_stats(config).await {
        Ok(response) => response
            .pools
            .into_iter()
            .filter_map(|stats| PoolLang::from_language(&stats.language).map(|lang| (lang, stats)))
            .collect(),
        Err(error) => {
            tracing::debug!("[runtime::pool] pool stats are unavailable: {error}");
            Vec::new()
        }
    }
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
