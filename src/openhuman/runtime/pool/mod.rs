//! Shared, bounded pools of long-lived `node` / `python` worker processes that
//! execute inline code jobs for skill runs and the `node_exec` agent tool —
//! instead of forking one interpreter child per execution (issue #5106).
//!
//! ## Why
//!
//! A single JS skill step spawns a `node` child at ~72–75 MB RSS. At the
//! opencompany target (100–1000 live agents in 2 GB / 2 vCPU) those per-run
//! interpreter children are the biggest budget breaker. Sharing a small bounded
//! pool of warm workers turns *K concurrent skill runs → K interpreters* into
//! *K concurrent skill runs → ~one pooled worker*, trading a little latency
//! (work beyond the pool size queues) for a large, flat memory floor.
//!
//! ## Shape
//!
//! * [`worker`] — one warm interpreter child speaking newline-delimited JSON.
//! * [`pool`] — the bounded [`LangPool`](pool::LangPool): semaphore-gated
//!   concurrency, queue backpressure, idle-TTL reaping, recycle-after-N-jobs,
//!   plus the process-global registry keyed per language.
//! * [`node`] / [`python`] — language backends that resolve the interpreter,
//!   materialise the harness script, and submit inline jobs.
//! * [`env`] — allow-listed worker environment + once-per-process harness write.
//!
//! The whole subsystem is an **optimisation seam**: `runtime_pool.enabled =
//! false` (or a per-language flag) reverts callers to their legacy per-call
//! spawn with no behavioural change.

pub mod env;
pub mod node;
// `module_inception` is a byproduct of the domain-family reorg: the parent was
// renamed from `runtime_pool` to `runtime/pool`, which shortened it to match this
// long-standing inner module. Renaming the inner module would be a real rename
// on top of a pure move, so it is allowed here and left as follow-up.
#[allow(clippy::module_inception)]
pub mod pool;
pub mod protocol;
pub mod python;
pub mod types;
pub mod worker;

pub(crate) use env::{base_env, ensure_worker_script};
pub use pool::{all_stats, LangPool, PoolRunError, PoolStats};
pub use types::{PoolExecOutcome, PoolLang, PoolSettings};
