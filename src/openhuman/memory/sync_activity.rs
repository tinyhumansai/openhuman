//! What is syncing right now, remembered from the stage stream.
//!
//! `memory_sources.status_list` answers "how far has this source got" from the
//! chunk store, and the store knows nothing about a run in flight: a connector
//! still paging Gmail in its detached task has written nothing yet, and the
//! row reads idle. The Sources screen kept the live state for itself, in
//! component state fed by the socket — and lost it the moment the screen
//! unmounted, so a tab change mid-sync looked like the sync had stopped
//! (openhuman#6019).
//!
//! This is the process's own memory of that stream. A bus subscriber beside
//! [`super::sync_events_bridge`] records every `MemorySyncStageChanged` that
//! names a source: a non-terminal stage puts the source in flight, `completed`
//! or `failed` takes it out. The status list reads it, so a screen that mounts
//! mid-run, or an app that reloads, is told where the run is instead of
//! guessing from a chunk count.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use async_trait::async_trait;
use tinybus::{EventHandler, SubscriptionHandle};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

/// One source's run in flight, as the stream last described it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSync {
    /// The last non-terminal stage seen — `running` for a connector sync,
    /// the per-item stages for a reader-backed one.
    pub stage: String,
    /// The stage's free-text detail, when it carried one.
    pub detail: Option<String>,
    /// When the entry was last written, epoch milliseconds.
    pub updated_at_ms: i64,
}

/// An entry silent for longer than this without a terminal stage is no run
/// at all.
///
/// The bus delivers locally over a broadcast channel that drops under lag, so
/// a `completed` can go missing, and without a ceiling that lost stage would
/// pin a "running" bar to the row for the life of the process. A live run is
/// never silent this long: the connector loop publishes a stage after every
/// pass, and one pass is bounded by the slow call's fifteen-minute deadline
/// (`modules::connectors::call_slow`), so the ceiling measures silence, not
/// the run.
const STALE_AFTER_MS: i64 = 30 * 60 * 1_000;

static LIVE: OnceLock<Mutex<HashMap<String, LiveSync>>> = OnceLock::new();
static HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

fn live() -> &'static Mutex<HashMap<String, LiveSync>> {
    LIVE.get_or_init(Default::default)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The two stages after which a source is no longer in flight.
pub(crate) fn is_terminal(stage: &str) -> bool {
    matches!(stage, "completed" | "failed")
}

/// Record one stage for `source_id`.
pub(crate) fn note_stage(source_id: &str, stage: &str, detail: Option<&str>) {
    note_stage_at(source_id, stage, detail, now_ms());
}

/// Drop every entry that has gone silent past the ceiling.
///
/// `live_sync_at` already answers `None` for a stale entry; this is what keeps
/// the map from holding it — and its strings — for the rest of the process. A
/// source removed mid-run is never asked about again, so its entry would
/// otherwise never leave.
fn prune_stale_at(map: &mut HashMap<String, LiveSync>, now_ms: i64) {
    map.retain(|_, entry| now_ms.saturating_sub(entry.updated_at_ms) <= STALE_AFTER_MS);
}

/// Sweep stale entries now. The status list calls it on a batch with no
/// sources, the one shape that would otherwise never touch the map.
pub fn prune_stale() {
    let mut map = live().lock().unwrap_or_else(PoisonError::into_inner);
    prune_stale_at(&mut map, now_ms());
}

fn note_stage_at(source_id: &str, stage: &str, detail: Option<&str>, at_ms: i64) {
    let mut map = live().lock().unwrap_or_else(PoisonError::into_inner);
    prune_stale_at(&mut map, at_ms);
    if is_terminal(stage) {
        map.remove(source_id);
    } else {
        map.insert(
            source_id.to_string(),
            LiveSync {
                stage: stage.to_string(),
                detail: detail.map(str::to_string),
                updated_at_ms: at_ms,
            },
        );
    }
}

/// The run in flight for `source_id`, if the stream has one that is not stale.
pub fn live_sync(source_id: &str) -> Option<LiveSync> {
    live_sync_at(source_id, now_ms())
}

fn live_sync_at(source_id: &str, now_ms: i64) -> Option<LiveSync> {
    let mut map = live().lock().unwrap_or_else(PoisonError::into_inner);
    prune_stale_at(&mut map, now_ms);
    map.get(source_id).cloned()
}

/// Subscribe the tracker to the bus, once. Registered beside the sync-stage
/// bridge so every host that serves the stream also remembers it.
pub fn register() {
    if HANDLE.get().is_some() {
        return;
    }
    match BUS.subscribe(Arc::new(SyncActivityTracker)) {
        Some(handle) => {
            let _ = HANDLE.set(handle);
            log::debug!("[event_bus] memory sync activity tracker registered");
        }
        None => log::warn!(
            "[event_bus] failed to register memory sync activity tracker — bus not initialized"
        ),
    }
}

struct SyncActivityTracker;

#[async_trait]
impl EventHandler<DomainEvent> for SyncActivityTracker {
    fn name(&self) -> &str {
        "memory::sync_activity_tracker"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // Only a stage that names its memory-source row can be shown on that
        // row; an unattributed one (a connection bootstrapped before its entry
        // exists) has nowhere to go and is left alone.
        if let DomainEvent::MemorySyncStageChanged {
            stage,
            detail,
            source_id: Some(source_id),
            ..
        } = event
        {
            note_stage(source_id, stage, detail.as_deref());
        }
    }
}

#[cfg(test)]
#[path = "sync_activity_tests.rs"]
mod tests;
