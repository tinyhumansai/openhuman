//! Paying the first lookup's cold start before anyone is waiting on it.
//!
//! The first `fast_retrieve` after boot is not like the ones that follow. The
//! module asks the host for spaCy entities, and the host's Python server may
//! still have to start and load its model; the query embedding opens the
//! embedder's first connection; the tree store is read cold. Measured on the
//! desktop (2026-09-05): the first question after launch took more than 5 s
//! and lost its block to the budget, the second took ~3 s. The user's first
//! question after launch is exactly the one that should not be answered "not
//! stored".
//!
//! So the host runs one throwaway retrieval as soon as the memory module is
//! loaded, off every request path, with a generous bound of its own. Whatever
//! it warms — the Python server, the embedder connection, the store's page
//! cache — is warm for the first real turn. Nothing depends on its result:
//! a signed-out embedder, a driver without retrieval, or a timeout each end
//! in one log line.

use super::source::{AutoRecallSource, GuardSource};
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::retrieval::FastRetrieveQuery;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the warm-up may take before it is abandoned. Wide, because this
/// is the call that absorbs the Python server start and the model load.
pub const WARM_UP_BUDGET: Duration = Duration::from_secs(60);

/// A query that opens every path a real lookup uses — entity extraction, a
/// query embedding, a tree walk — without matching anything in particular.
const WARM_UP_QUERY: &str = "warm up the memory retrieval path";

/// What a warm-up run found out, for the log and for tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarmUpOutcome {
    /// The retrieval answered (with however many hits) inside the budget.
    Warmed { elapsed_ms: u128 },
    /// The retrieval failed; the failure is logged and nothing is retried.
    Failed { elapsed_ms: u128 },
    /// The budget expired first.
    TimedOut,
}

/// Run one warm-up retrieval against `source`, bounded by `budget`.
pub async fn warm_up(source: &dyn AutoRecallSource, budget: Duration) -> WarmUpOutcome {
    let started = Instant::now();
    let query = FastRetrieveQuery {
        limit: 1,
        ..FastRetrieveQuery::default()
    };
    match tokio::time::timeout(budget, source.fast_retrieve(WARM_UP_QUERY, query)).await {
        Ok(Ok(response)) => {
            let elapsed_ms = started.elapsed().as_millis();
            log::info!(
                "[auto_recall] warm-up done elapsed_ms={elapsed_ms} hits={} total={}",
                response.hits.len(),
                response.total
            );
            WarmUpOutcome::Warmed { elapsed_ms }
        }
        Ok(Err(err)) => {
            let elapsed_ms = started.elapsed().as_millis();
            // Expected while signed out (the embedder has no session) and on a
            // driver without retrieval; the first real turn reports the same
            // condition on its own line, so this stays at debug.
            log::debug!("[auto_recall] warm-up failed after {elapsed_ms}ms: {err}");
            WarmUpOutcome::Failed { elapsed_ms }
        }
        Err(_elapsed) => {
            log::warn!(
                "[auto_recall] warm-up exceeded {budget:?}; the first lookup will pay the cold start"
            );
            WarmUpOutcome::TimedOut
        }
    }
}

/// Bind the memory driver `config` selects and run [`warm_up`] through its
/// guard. `None` when the driver cannot be bound — logged, never an error.
pub async fn warm_up_from_config(config: &Config) -> Option<WarmUpOutcome> {
    let binding = match crate::openhuman::memory::binding::for_config(config) {
        Ok(binding) => binding,
        Err(reason) => {
            log::debug!("[auto_recall] warm-up skipped: memory binding unavailable: {reason}");
            return None;
        }
    };
    let source = GuardSource::new(binding.guard());
    Some(warm_up(&source, WARM_UP_BUDGET).await)
}

/// Spawn the boot warm-up for the memory driver `config` binds.
///
/// Returns without doing anything when the lane is switched off — a host that
/// disabled `auto_recall` asked for no pre-turn retrieval work at all. Never
/// blocks the caller; the spawned task's own timeout only stops the wait, a
/// retrieval already in flight inside the module runs to completion.
pub fn spawn_at_boot(config: Arc<Config>) {
    if !config.subsystems.memory.hooks.auto_recall {
        log::debug!("[auto_recall] warm-up skipped: hooks.auto_recall is off");
        return;
    }
    tokio::spawn(async move {
        warm_up_from_config(&config).await;
    });
}

#[cfg(test)]
#[path = "warm_tests.rs"]
mod tests;
