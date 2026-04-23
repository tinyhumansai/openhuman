//! Background ticker that drives `SessionManager::tick` on an interval.
//!
//! Modeled after `subconscious::engine::run` — a plain `tokio::spawn`
//! loop with a fixed interval. Not a bus subscriber because A3 writes
//! chronicle_events synchronously to SQLite without bus emission; polling
//! is simpler and lossless.
//!
//! `TICK_INTERVAL` is deliberately shorter than the idle-gap threshold
//! (5m) and the app-switch threshold (3m) so boundary detection stays
//! snappy without busy-looping.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::openhuman::life_capture::chronicle::sessions::manager::SessionManager;
use crate::openhuman::life_capture::index::PersonalIndex;

/// Poll cadence. 60s is well under both the idle-5m and switch-3m
/// thresholds so boundaries are detected within one tick of the event
/// that triggers them.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);

/// Spawn the ticker task. Returns the `JoinHandle` for diagnostics and
/// tests; the caller may drop it — tokio `JoinHandle` drop detaches the
/// task, it does not cancel. The loop runs until process exit.
pub fn spawn(idx: Arc<PersonalIndex>) -> tokio::task::JoinHandle<()> {
    let manager = Arc::new(Mutex::new(SessionManager::new()));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        // Skip the immediate firstshot; the first chronicle tick has
        // nothing to process on a cold start anyway.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let mut mgr = manager.lock().await;
            if let Err(e) = mgr.tick(&idx).await {
                // Do not bring down the loop on a single failure — log and
                // retry on the next tick. Most errors are transient SQL
                // contention; a persistent failure will be visible in logs.
                tracing::warn!(
                    target: "life_capture::chronicle::sessions",
                    error = %e,
                    "session manager tick failed",
                );
            }
        }
    })
}
