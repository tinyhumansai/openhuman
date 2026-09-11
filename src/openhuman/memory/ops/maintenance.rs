//! Upkeep asked of the bound memory driver rather than of TinyCortex.
//!
//! The operations here are scheduler-driven housekeeping — the `Maintenance`
//! capability family. Each one used to be a direct call into
//! `tinymemory_core::queue`, which is a second door into the memory subsystem
//! that skips the capability filter and the wire error table
//! (`memory::direct_engine_refs_tests` is the ratchet over the ones that
//! remain).
//!
//! Everything here is **best-effort by contract**. These are called after a
//! save has already succeeded, so a failure must not fail the caller's
//! operation — but it must not be silently swallowed either, which is why each
//! returns a `Result` for the call site to log rather than logging here and
//! handing back nothing.

use crate::openhuman::config::Config;
use crate::openhuman::memory::binding;

/// Ask the bound driver to enqueue re-embedding for content whose embedding is
/// missing or stale, and answer how much work that enqueued.
///
/// This is idempotent and coverage-gated at the driver: when the active
/// embedding signature has not actually changed there is nothing uncovered, so
/// it enqueues nothing and reports `0`. Calling it after every embedder-adjacent
/// save is therefore the intended usage, not a cost.
///
/// `0` from a driver that does not serve `Maintenance` means the same thing it
/// means from one that does — no work was enqueued — which is true of a driver
/// with no re-embedding to do.
///
/// # Errors
///
/// Whatever the driver reports. Callers surface it and carry on; none of them
/// can undo the save that preceded it.
pub async fn reembed(config: &Config) -> Result<u64, String> {
    let binding = binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        log::debug!(
            "[memory::maintenance] reembed: driver '{}' does not serve Maintenance; nothing enqueued",
            binding.driver_id()
        );
        return Ok(0);
    };
    let report = maintenance
        .reembed()
        .await
        .map_err(|error| format!("reembed: {error}"))?;
    log::debug!(
        "[memory::maintenance] reembed: driver '{}' examined={} enqueued={}",
        binding.driver_id(),
        report.examined,
        report.changed
    );
    Ok(report.changed)
}

/// Ask the bound driver to give terminally-failed queue work another attempt.
///
/// Answers how many jobs were requeued. The driver wakes whatever drains the
/// queue as part of the same operation, so there is no second call to forget:
/// jobs flipped back to `ready` that then wait for the next scheduled window
/// look, to the user who pressed the button, exactly like a retry that did
/// nothing.
///
/// A driver that does not serve `Maintenance` reports `0`, which is what it
/// requeued.
///
/// # Errors
///
/// Whatever the driver reports.
pub async fn retry_failed(config: &Config) -> Result<u64, String> {
    let binding = binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        log::debug!(
            "[memory::maintenance] retry_failed: driver '{}' does not serve Maintenance; nothing requeued",
            binding.driver_id()
        );
        return Ok(0);
    };
    let report = maintenance
        .retry_failed()
        .await
        .map_err(|error| format!("retry_failed: {error}"))?;
    log::debug!(
        "[memory::maintenance] retry_failed: driver '{}' examined={} requeued={}",
        binding.driver_id(),
        report.examined,
        report.changed
    );
    Ok(report.changed)
}

/// Log-and-continue wrapper for the call sites that have nothing to report the
/// count to.
///
/// The engine call this replaced swallowed its own errors the same way, so this
/// keeps those sites behaving identically instead of making them grow error
/// handling they have nowhere to put.
pub async fn reembed_best_effort(config: &Config, context: &str) {
    match reembed(config).await {
        Ok(enqueued) => {
            log::debug!("[memory::maintenance] {context}: enqueued {enqueued} re-embedding job(s)")
        }
        Err(error) => {
            log::warn!("[memory::maintenance] {context}: re-embed enqueue failed: {error}")
        }
    }
}

#[cfg(test)]
#[path = "maintenance_tests.rs"]
mod tests;
