//! The wire payload for the memory subsystem's `user_error` web-channel event.
//!
//! `tinymemory_core::tree::health::user_error` decides *that* a local-runtime
//! failure must reach the user; this module decides *what goes on the wire*.
//! The split is the same one the rest of the extraction follows — web channels,
//! and the socket the payload rides, are host surface.
//!
//! Both halves key on `LOCAL_MODEL_UNAVAILABLE_KIND`, which lives in the
//! contract crate so the two cannot drift apart silently.

use tinymemory_api::host::{
    LOCAL_MODEL_UNAVAILABLE_KIND, MEMORY_USER_ERROR_SOURCE, STORE_CORRUPT_KIND,
};

use crate::core::socketio::WebChannelEvent;

/// The metadata-only `user_error` payload for an unusable local embedding
/// runtime. Built separately from the publish so the no-leak contract is
/// unit-testable without a live socket.
///
/// Metadata only, exactly like the cron producer: a stable `kind` token in
/// `error_type` plus `error_source`, and never the raw provider text, the model
/// id, or the configured endpoint (which can carry a private host).
pub(crate) fn local_model_unavailable_user_error() -> WebChannelEvent {
    WebChannelEvent {
        event: "user_error".to_string(),
        // Every socket auto-joins the "system" room, so this reaches all
        // connected clients rather than one chat session.
        client_id: "system".to_string(),
        error_type: Some(LOCAL_MODEL_UNAVAILABLE_KIND.to_string()),
        error_source: Some(MEMORY_USER_ERROR_SOURCE.to_string()),
        ..Default::default()
    }
}

/// Broadcast the local-runtime user error to every connected client.
///
/// Called from the `MemoryEvent::LocalModelUnavailable` arm of the sink in
/// [`crate::openhuman::memory::host`]. `origin` names the producer
/// (`health_gate` / `embed_classify`) and is logged, never sent.
pub(crate) fn publish_local_model_unavailable_user_error(origin: &str) {
    log::debug!(
        "[memory::host] action=broadcast_user_error kind={LOCAL_MODEL_UNAVAILABLE_KIND} \
         source={MEMORY_USER_ERROR_SOURCE} origin={origin}"
    );
    crate::openhuman::web_chat::publish_web_channel_event(local_model_unavailable_user_error());
}

/// The metadata-only `user_error` payload for a corrupt-and-quarantined
/// memory-tree store (openhuman#5820). Same no-leak contract as the
/// local-model payload above: a stable `kind` token plus the source, never the
/// filesystem path (which is logged host-side instead).
pub(crate) fn store_corrupt_quarantined_user_error() -> WebChannelEvent {
    WebChannelEvent {
        event: "user_error".to_string(),
        // Every socket auto-joins the "system" room, so this reaches all
        // connected clients rather than one chat session.
        client_id: "system".to_string(),
        error_type: Some(STORE_CORRUPT_KIND.to_string()),
        error_source: Some(MEMORY_USER_ERROR_SOURCE.to_string()),
        ..Default::default()
    }
}

/// Text classifier for a corrupt chunk store, for errors that crossed the bus
/// and only exist as strings (a `MemoryError`'s rendered message).
///
/// The two phrases are SQLite's own renderings of `SQLITE_CORRUPT` (code 11)
/// and `SQLITE_NOTADB` (code 26) and survive every flattening the wire
/// applies. Mirrors the typed classifier inside `tinymemory-core` — the
/// module side classifies before the error is stringified; this is the
/// Host-owned `error_type` for "the memory module failed to load".
///
/// Host-owned (not a `tinymemory_api::host` constant) because the engine can
/// never emit it: a module that failed to load has no code running to report
/// anything. Only the host's loader observes this state, so the constant lives
/// with the only producer.
pub(crate) const MEMORY_MODULE_UNAVAILABLE_KIND: &str = "memory_module_unavailable";

/// The metadata-only `user_error` payload for a memory module that failed to
/// load. Same no-leak contract as its siblings: a stable kind plus the source,
/// never the loader's raw reason (which can carry URLs and filesystem paths).
pub(crate) fn memory_module_unavailable_user_error() -> WebChannelEvent {
    WebChannelEvent {
        event: "user_error".to_string(),
        client_id: "system".to_string(),
        error_type: Some(MEMORY_MODULE_UNAVAILABLE_KIND.to_string()),
        error_source: Some(MEMORY_USER_ERROR_SOURCE.to_string()),
        ..Default::default()
    }
}

/// Broadcast the module-unavailable user error once per process.
///
/// Once-guarded like [`notice_corrupt_store_once`]: the loader caches a load
/// failure as terminal, so every subsequent memory call re-observes the same
/// state, and a per-call broadcast would be a banner storm. `reason` is the
/// loader's raw message — logged for the operator, never sent.
pub(crate) fn notice_memory_module_unavailable_once(reason: &str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        log::error!(
            "[memory::host] action=broadcast_user_error kind={MEMORY_MODULE_UNAVAILABLE_KIND}              source={MEMORY_USER_ERROR_SOURCE} reason={reason}"
        );
        crate::openhuman::web_chat::publish_web_channel_event(
            memory_module_unavailable_user_error(),
        );
    });
}

/// host-side fallback for paths that only ever see text.
pub(crate) fn is_corrupt_store_error(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("database disk image is malformed") || msg.contains("file is not a database")
}

/// Once-per-process corrupt-store notice for wire-text detectors
/// (openhuman#5820).
///
/// The archivist sees the corruption as one failed call per segment — 747
/// warns in the incident — so its escalation must not become 747 notices.
/// The engine's own quarantine still publishes the authoritative
/// [`MemoryEvent::StoreCorruptQuarantined`](tinymemory_api::host::MemoryEvent)
/// notice (un-latched, once per actual quarantine); this latch only bounds the
/// early-warning duplicate from paths that detect the damage before the
/// engine's recovery runs.
pub(crate) fn notice_corrupt_store_once(origin: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WIRE_CORRUPT_NOTICED: AtomicBool = AtomicBool::new(false);
    if WIRE_CORRUPT_NOTICED.swap(true, Ordering::Relaxed) {
        return;
    }
    publish_store_corrupt_user_error(origin, None);
}

/// Broadcast the corrupt-store user error and log the quarantined path
/// prominently (openhuman#5820 item 4 — 172 MB of a user's indexed memory
/// sitting on disk must never be a secret between the app and itself).
///
/// Called from the `MemoryEvent::StoreCorruptQuarantined` arms of both event
/// sinks — the in-process one in [`crate::openhuman::memory::host`] and the
/// module bridge in `crate::openhuman::modules::memory_host` — so the notice
/// fires whichever engine detected the damage. `origin` names the detecting
/// path; `quarantined_path` is the preserved copy, logged and never sent.
pub(crate) fn publish_store_corrupt_user_error(origin: &str, quarantined_path: Option<&str>) {
    match quarantined_path {
        Some(path) => log::error!(
            "[memory::host] the memory-tree store was corrupt and has been quarantined \
             (detected by {origin}). The damaged file is PRESERVED at {path} — recovery \
             tooling (e.g. `sqlite3 .recover`) can likely salvage most of it. The rebuilt \
             store is empty; re-sync memory sources to repopulate"
        ),
        None => log::error!(
            "[memory::host] the memory-tree store was corrupt and has been quarantined \
             (detected by {origin}); the damaged file is preserved beside the store as \
             chunks.db.corrupt-<timestamp>. The rebuilt store is empty; re-sync memory \
             sources to repopulate"
        ),
    }
    crate::openhuman::web_chat::publish_web_channel_event(store_corrupt_quarantined_user_error());
}

#[cfg(test)]
#[path = "user_error_tests.rs"]
mod tests;
