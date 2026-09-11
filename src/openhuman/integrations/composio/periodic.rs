//! Periodic Composio connection sync, driven by this host.
//!
//! # Why this is host code now, and not a shim
//!
//! This used to be `pub use memory_sync::composio::periodic::*` — engine code
//! (`tinymemory_core::sync::composio::periodic`) running in this process
//! against the in-process engine. tinymemory v1.13.4 deleted that module
//! along with the rest of the in-process Composio pipeline (reaching a
//! connected account needs a credential the engine must not hold), and the
//! `tinymemory-module` this host loads never grew a replacement — its own
//! `crate::composio` bridge, referenced in its own docs, does not exist in
//! the pinned release either. So nothing anywhere in this build runs a
//! periodic Composio sync unless this file does it.
//!
//! The mechanics are unchanged in spirit: on a tick, list every active
//! connection, skip toolkits/connections that synced recently enough, and
//! read+ingest the rest through [`super::ops::run_sync_pass`] — the same
//! `tinyconnectors` → `MemorySourceSink::accept_source_items` path every other
//! sync entry point in this domain uses.
//!
//! # What did not carry over
//!
//! The deleted engine tracked cost (`actions_called`, `provider_cost_usd`)
//! and a persisted per-connection cursor across restarts. Neither survives
//! here: `ConnectorSyncResponse` carries no cost field, and "due to sync" is
//! tracked in an in-process map rather than on disk, so a process restart
//! forgets when a connection last ran and syncs it on the next tick rather
//! than waiting out the remainder of its interval. Both are worth knowing if
//! Composio spend or sync cadence ever needs auditing across a restart.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::providers::native_provider_sync_interval_secs;

/// How often the loop wakes up to check whether anything is due. Independent
/// of any one toolkit's own interval — this is just the polling granularity.
const TICK_INTERVAL: Duration = Duration::from_secs(60);

/// Guards [`start_periodic_sync`] so a second call (e.g. a second embedder
/// boot path) does not spawn a second loop.
static STARTED: OnceLock<()> = OnceLock::new();

/// When each `(toolkit, connection_id)` last completed a sync — either the
/// periodic loop's own run or a trigger-driven one recorded through
/// [`record_sync_success`].
fn last_synced() -> &'static Mutex<HashMap<(String, String), Instant>> {
    static LAST_SYNCED: OnceLock<Mutex<HashMap<(String, String), Instant>>> = OnceLock::new();
    LAST_SYNCED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record that `(toolkit, connection_id)` just synced successfully outside
/// the periodic loop (a trigger-driven sync), so the next tick does not
/// immediately re-run it.
pub fn record_sync_success(toolkit: &str, connection_id: &str) {
    let key = (toolkit.to_ascii_lowercase(), connection_id.to_string());
    if let Ok(mut map) = last_synced().lock() {
        map.insert(key, Instant::now());
    }
}

/// Start the periodic Composio sync loop. Idempotent — a second call is a
/// no-op.
pub fn start_periodic_sync() {
    if STARTED.set(()).is_err() {
        log::debug!("[composio:periodic] start_periodic_sync called again; loop already running");
        return;
    }
    log::info!("[composio:periodic] starting periodic Composio sync loop (tick={TICK_INTERVAL:?})");
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK_INTERVAL).await;
            if let Err(error) = run_one_tick().await {
                log::warn!("[composio:periodic] tick failed: {error}");
            }
        }
    });
}

/// One pass over every active connection: sync whichever toolkits are due.
async fn run_one_tick() -> Result<(), String> {
    let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;

    let connections =
        crate::openhuman::integrations::composio::ops::composio_list_connections(&config)
            .await
            .map_err(|error| format!("list_connections: {error}"))?
            .value
            .connections;

    for conn in connections {
        if !conn.is_active() {
            continue;
        }
        let toolkit = conn.normalized_toolkit();

        // A toolkit with no periodic interval opted out entirely — matches
        // `ComposioProvider::sync_interval_secs() -> None` in the deleted
        // engine, now answered by the catalog instead of a provider instance.
        let Some(interval_secs) = native_provider_sync_interval_secs(&toolkit) else {
            continue;
        };

        let key = (toolkit.clone(), conn.id.clone());
        let due = {
            let map = last_synced()
                .lock()
                .map_err(|_| "sync bookkeeping lock poisoned".to_string())?;
            match map.get(&key) {
                Some(last) => last.elapsed() >= Duration::from_secs(interval_secs),
                None => true,
            }
        };
        if !due {
            continue;
        }

        match crate::openhuman::integrations::composio::ops::run_sync_within_budget(
            &config, &toolkit, &conn.id, "periodic",
        )
        .await
        {
            Ok(pass) => {
                log::debug!(
                    "[composio:periodic] toolkit={toolkit} connection={} synced \
                     records_read={} written={} more_pending={}",
                    conn.id,
                    pass.records_read,
                    pass.written,
                    pass.more_pending
                );
                record_sync_success(&toolkit, &conn.id);
            }
            Err(error) => {
                log::warn!(
                    "[composio:periodic] toolkit={toolkit} connection={} sync failed: {error}",
                    conn.id
                );
                // Record the attempt regardless of outcome: a connection that
                // keeps failing must not be retried every tick, which would
                // turn one broken account into a hot loop against the module.
                record_sync_success(&toolkit, &conn.id);
            }
        }
    }

    Ok(())
}
