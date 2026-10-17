//! What the `tinychannels` module may touch through the storage bus.
//!
//! The collections are the module's own declaration (`tinychannels::storage`
//! and `tinychannels::remote::session_store` in tinyhumansai/tinychannels,
//! contract v21), restated here because a host never links a module's
//! implementation library: a grant is a reviewable build-time decision, like
//! the registry's digests. When the module's declaration changes, this table
//! changes in the same re-pin; the test in `storage_grant_tinychannels_tests.rs`
//! pins the shape.
//!
//! * delivery queue: pending, failed and terminal-result records behind
//!   required deliveries (replacing the `Save/LoadPendingDelivery` callbacks);
//! * remote control: chat to thread bindings, in-flight flags and the legacy
//!   import marker (replacing the remote-session JSON map);
//! * WhatsApp Web: the linked-device session, so pairing survives a restart.
//!
//! No streams, blobs or secrets: the module keeps none there. The iMessage
//! `chat.db` reader is an external read-only database, not storage.

use tinystoragedrivers::CollectionSpec;

use super::storage_grants::StorageGrant;

/// The registry id the grant is declared under.
pub(super) const MODULE_ID: &str = "tinychannels";

/// Delivery entries awaiting a terminal outcome.
const DELIVERY_PENDING: &str = "channels_delivery_pending";
/// Permanently failed delivery entries.
const DELIVERY_FAILED: &str = "channels_delivery_failed";
/// Idempotent terminal delivery results.
const DELIVERY_RESULTS: &str = "channels_delivery_results";
/// Chat to host-thread bindings.
const REMOTE_BINDINGS: &str = "channel_remote_bindings";
/// Chats with a turn in flight.
const REMOTE_BUSY: &str = "channel_remote_busy";
/// The one-time legacy import marker of the remote-session files.
const REMOTE_META: &str = "channel_remote_meta";
/// The WhatsApp Web session backend's collections.
const WHATSAPP: [&str; 8] = [
    "whatsapp_device",
    "whatsapp_identity",
    "whatsapp_session",
    "whatsapp_prekey",
    "whatsapp_signed_prekey",
    "whatsapp_sender_key",
    "whatsapp_sync_key",
    "whatsapp_lid_mapping",
];

/// The grant the host serves the `tinychannels` module under.
pub(super) fn grant() -> StorageGrant {
    let collections = [
        DELIVERY_PENDING,
        DELIVERY_FAILED,
        DELIVERY_RESULTS,
        REMOTE_BINDINGS,
        REMOTE_BUSY,
        REMOTE_META,
    ]
    .into_iter()
    .chain(WHATSAPP)
    .map(CollectionSpec::new)
    .collect();
    StorageGrant {
        collections,
        stream_prefixes: Vec::new(),
        blob_prefixes: Vec::new(),
        secret_prefixes: Vec::new(),
        writable: true,
    }
}

#[cfg(test)]
#[path = "storage_grant_tinychannels_tests.rs"]
mod tests;
