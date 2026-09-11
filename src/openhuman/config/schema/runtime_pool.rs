//! Shared language-runtime pool configuration (`[runtime_pool]`).
//!
//! Controls the bounded pools of long-lived `node` / `python` worker processes
//! that execute inline code jobs for skill runs and the `node_exec` agent tool
//! instead of forking one interpreter child per execution (issue #5106).
//!
//! ## Why this exists
//!
//! At the opencompany deployment target (100–1000 live agents in a
//! 2 GB / 2 vCPU box) a per-run interpreter child is the single biggest budget
//! breaker: a single JS skill step spawns a `node` child at ~72–75 MB RSS.
//! Sharing a small, bounded pool of warm workers turns "K concurrent skill runs
//! → K interpreters" into "K concurrent skill runs → ~one pooled worker", at
//! the cost of serialising work beyond the pool size (surfaced as queue wait).
//!
//! ## Kill switch
//!
//! `enabled = false` (globally, or per language) reverts callers to the legacy
//! per-call spawn path with **no** behavioural change — the pool is purely an
//! optimisation seam and must always be safe to turn off.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `[runtime_pool]` — top-level switch plus per-language pool tuning.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RuntimePoolConfig {
    /// Master switch. When `false`, no pool is started and every caller falls
    /// back to spawning a fresh interpreter child per execution (legacy
    /// behaviour). Per-language `enabled` flags gate each language on top of
    /// this.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Node.js worker pool.
    #[serde(default)]
    pub node: RuntimePoolLangConfig,
    /// Python worker pool.
    #[serde(default)]
    pub python: RuntimePoolLangConfig,
}

/// Per-language pool tuning. Applies identically to the node and python pools.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RuntimePoolLangConfig {
    /// Whether this language routes through the pool. `None` (unset) means
    /// "use the per-language default" — resolved via [`Self::is_enabled`]:
    /// **node defaults on** (worker_thread isolation makes reuse safe),
    /// **python defaults off** (in-process reuse can leak globals across jobs,
    /// so it stays opt-in until stronger isolation lands). `false`/`true`
    /// override explicitly.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Maximum number of concurrently-resident worker processes. Concurrent
    /// jobs beyond this bound **queue** rather than fork a new interpreter —
    /// this is the whole point of the pool. Clamped to at least 1 at read time.
    #[serde(default = "default_max_workers")]
    pub max_workers: usize,
    /// Idle time-to-live (seconds). A worker that has served no job for this
    /// long is reaped so an idle fleet pays zero interpreter RSS. `0` disables
    /// idle reaping (workers live until recycled or the process exits).
    #[serde(default = "default_idle_ttl_secs")]
    pub idle_ttl_secs: u64,
    /// Recycle a worker after it has completed this many jobs, bounding
    /// state/heap contamination across otherwise-isolated runs. `0` disables
    /// job-count recycling.
    #[serde(default = "default_recycle_after_jobs")]
    pub recycle_after_jobs: u64,
    /// Maximum number of jobs allowed to wait in the queue for a free worker
    /// before new submissions are rejected with backpressure (rather than
    /// growing memory unboundedly). Clamped to at least 1 at read time.
    #[serde(default = "default_max_queue_depth")]
    pub max_queue_depth: usize,
}

impl RuntimePoolLangConfig {
    /// Whether this language routes through the pool, resolving an unset
    /// `enabled` to the caller-supplied per-language default (node → `true`,
    /// python → `false`). An explicit `enabled = true/false` always wins.
    pub fn is_enabled(&self, default: bool) -> bool {
        self.enabled.unwrap_or(default)
    }

    /// Effective worker count, never zero.
    pub fn effective_max_workers(&self) -> usize {
        self.max_workers.max(1)
    }

    /// Effective queue depth, never zero.
    pub fn effective_max_queue_depth(&self) -> usize {
        self.max_queue_depth.max(1)
    }
}

fn default_true() -> bool {
    true
}

fn default_max_workers() -> usize {
    2
}

fn default_idle_ttl_secs() -> u64 {
    60
}

fn default_recycle_after_jobs() -> u64 {
    100
}

fn default_max_queue_depth() -> usize {
    256
}

impl Default for RuntimePoolConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            node: RuntimePoolLangConfig::default(),
            python: RuntimePoolLangConfig::default(),
        }
    }
}

impl Default for RuntimePoolLangConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            max_workers: default_max_workers(),
            idle_ttl_secs: default_idle_ttl_secs(),
            recycle_after_jobs: default_recycle_after_jobs(),
            max_queue_depth: default_max_queue_depth(),
        }
    }
}

#[cfg(test)]
#[path = "runtime_pool_tests.rs"]
mod tests;
