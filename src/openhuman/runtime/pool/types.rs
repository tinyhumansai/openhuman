//! Public types callers of the runtime pool see.

use std::time::Duration;

use crate::openhuman::config::RuntimePoolLangConfig;

/// Language a pool serves. Drives which harness script + interpreter launches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolLang {
    Node,
    Python,
}

impl PoolLang {
    pub fn id(self) -> &'static str {
        match self {
            PoolLang::Node => "node",
            PoolLang::Python => "python",
        }
    }
}

/// The result of running one job on a pooled worker. Mirrors the fields a
/// per-call `std::process` spawn would have exposed, plus `queue_wait` so
/// callers can surface backpressure in run logs (a DoD requirement of #5106).
#[derive(Debug, Clone)]
pub struct PoolExecOutcome {
    pub stdout: String,
    pub stderr: String,
    /// `0` on success, non-zero when the job threw / exited non-zero.
    pub exit_code: Option<i32>,
    /// The job hit its soft deadline and was aborted.
    pub timed_out: bool,
    /// Wall-clock the job itself took inside the worker.
    pub elapsed: Duration,
    /// How long the submission waited for a free worker (queue backpressure).
    pub queue_wait: Duration,
}

impl PoolExecOutcome {
    /// A job "succeeded" when it ran to completion with a zero (or absent) exit
    /// code and did not time out.
    pub fn success(&self) -> bool {
        !self.timed_out && matches!(self.exit_code, None | Some(0))
    }
}

/// The knobs a single language pool reads from config, snapshotted at pool
/// construction. Kept as a plain owned struct so the pool never re-reads config
/// mid-flight.
#[derive(Debug, Clone)]
pub struct PoolSettings {
    pub max_workers: usize,
    pub idle_ttl: Option<Duration>,
    pub recycle_after_jobs: u64,
    pub max_queue_depth: usize,
}

impl PoolSettings {
    /// Derive the effective settings from a per-language config block, applying
    /// the same "never zero" clamps the config getters use.
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
mod tests {
    use super::*;

    #[test]
    fn outcome_success_semantics() {
        let base = PoolExecOutcome {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            timed_out: false,
            elapsed: Duration::ZERO,
            queue_wait: Duration::ZERO,
        };
        assert!(base.success());
        assert!(!PoolExecOutcome {
            exit_code: Some(1),
            ..base.clone()
        }
        .success());
        assert!(!PoolExecOutcome {
            timed_out: true,
            ..base.clone()
        }
        .success());
    }

    #[test]
    fn settings_disable_idle_reap_on_zero() {
        let cfg = RuntimePoolLangConfig {
            enabled: Some(true),
            max_workers: 3,
            idle_ttl_secs: 0,
            recycle_after_jobs: 5,
            max_queue_depth: 10,
        };
        let s = PoolSettings::from_lang_config(&cfg);
        assert_eq!(s.max_workers, 3);
        assert!(s.idle_ttl.is_none());
        assert_eq!(s.recycle_after_jobs, 5);
    }
}
