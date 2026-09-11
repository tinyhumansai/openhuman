//! The types callers of pooled execution see.

use std::time::Duration;

use tinyruntime_bus::{ExecResponse, Language};

use crate::openhuman::config::RuntimePoolLangConfig;

/// A language with a worker pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolLang {
    /// JavaScript on the managed Node.js toolchain.
    Node,
    /// Python.
    Python,
}

impl PoolLang {
    /// The identifier used in logs and status surfaces.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
        }
    }

    /// The bus language this pool serves.
    #[must_use]
    pub fn language(self) -> Language {
        match self {
            Self::Node => Language::nodejs(),
            Self::Python => Language::python(),
        }
    }

    /// The pool a bus language belongs to, if this build has one for it.
    ///
    /// A language this core ships no pool concept for is `None` rather than an
    /// error: the module routes whatever its configuration routes, and a status
    /// surface should skip an unfamiliar entry rather than fail rendering.
    #[must_use]
    pub fn from_language(language: &Language) -> Option<Self> {
        match language.as_str() {
            tinyruntime_bus::NODEJS => Some(Self::Node),
            tinyruntime_bus::PYTHON => Some(Self::Python),
            _ => None,
        }
    }
}

/// The result of running one job on a pooled worker.
///
/// Mirrors the fields a per-call spawn would have exposed, plus `queue_wait` so
/// callers can surface backpressure in run logs — a host that cannot tell a slow
/// job from a busy pool will tune the wrong thing.
#[derive(Debug, Clone)]
pub struct PoolExecOutcome {
    /// Everything the job wrote to standard output.
    pub stdout: String,
    /// Everything the job wrote to standard error.
    pub stderr: String,
    /// `0` on success, non-zero when the job threw or exited non-zero.
    pub exit_code: Option<i32>,
    /// The job hit its soft deadline and was aborted.
    pub timed_out: bool,
    /// Wall-clock the job itself took inside the worker.
    pub elapsed: Duration,
    /// How long the submission waited for a free worker.
    pub queue_wait: Duration,
}

impl PoolExecOutcome {
    /// Adapt a module reply.
    #[must_use]
    pub fn from_module(response: &ExecResponse) -> Self {
        Self {
            stdout: response.stdout.clone(),
            stderr: response.stderr.clone(),
            exit_code: response.exit_code,
            timed_out: response.timed_out,
            elapsed: Duration::from_millis(response.elapsed_ms),
            queue_wait: Duration::from_millis(response.queue_wait_ms),
        }
    }

    /// A job "succeeded" when it ran to completion with a zero (or absent) exit
    /// code and did not time out.
    #[must_use]
    pub fn success(&self) -> bool {
        !self.timed_out && matches!(self.exit_code, None | Some(0))
    }
}

/// The knobs a single language pool reads from config.
///
/// Kept as a plain owned struct so a caller never re-reads config mid-flight.
#[derive(Debug, Clone)]
pub struct PoolSettings {
    /// Concurrent workers.
    pub max_workers: usize,
    /// Retire a worker after this long idle, or never.
    pub idle_ttl: Option<Duration>,
    /// Retire a worker after this many jobs. `0` disables recycling.
    pub recycle_after_jobs: u64,
    /// Jobs allowed to queue beyond the worker slots.
    pub max_queue_depth: usize,
}

impl PoolSettings {
    /// Derive the effective settings from a per-language config block, applying
    /// the same "never zero" clamps the config getters use.
    #[must_use]
    pub fn from_lang_config(cfg: &RuntimePoolLangConfig) -> Self {
        Self {
            max_workers: cfg.effective_max_workers(),
            idle_ttl: if cfg.idle_ttl_secs == 0 {
                None
            } else {
                Some(Duration::from_secs(cfg.idle_ttl_secs))
            },
            recycle_after_jobs: cfg.recycle_after_jobs,
            max_queue_depth: cfg.effective_max_queue_depth(),
        }
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
