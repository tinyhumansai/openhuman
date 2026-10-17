//! What the `tinyconnectors` module may touch through the storage bus.
//!
//! The names are the module's own declaration (`tinyconnectors::state::
//! STORAGE_NEEDS` in tinyhumansai/tinyconnectors, v0.18.0), restated here
//! because a host never links a module's implementation library: a grant is a
//! reviewable build-time decision, like the registry's digests. When the
//! module's declaration changes, this table changes in the same re-pin; the
//! test in `storage_grant_tinyconnectors_tests.rs` pins the shape.
//!
//! * `composio-user-scopes`: one document per toolkit holding the user's scope
//!   preference (`{ "value": ... }`);
//! * `triggers/`: the trigger archive, one stream per UTC day
//!   (`triggers/<YYYY-MM-DD>`).
//!
//! No blobs or secrets: the module keeps none there.

use tinystoragedrivers::CollectionSpec;

use super::storage_grants::StorageGrant;

/// The registry id the grant is declared under.
pub(super) const MODULE_ID: &str = "tinyconnectors";

/// Per-toolkit scope preferences, one document each.
pub(super) const PREFS_COLLECTION: &str = "composio-user-scopes";
/// Prefix of every day's trigger-archive stream.
pub(super) const TRIGGER_STREAM_PREFIX: &str = "triggers/";

/// The grant the host serves the `tinyconnectors` module under.
pub(super) fn grant() -> StorageGrant {
    StorageGrant {
        collections: vec![CollectionSpec::new(PREFS_COLLECTION)],
        stream_prefixes: vec![TRIGGER_STREAM_PREFIX.into()],
        blob_prefixes: Vec::new(),
        secret_prefixes: Vec::new(),
        writable: true,
    }
}

#[cfg(test)]
#[path = "storage_grant_tinyconnectors_tests.rs"]
mod tests;
