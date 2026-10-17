//! What the `tinybox` module may touch through the storage bus.
//!
//! The names are the module's own (`tinybox_drivers` in
//! tinyhumansai/tinybox, v0.1.29), restated here because a host never links a
//! module's implementation library: a grant is a reviewable build-time
//! decision, like the registry's digests. When the module's declaration
//! changes, this table changes in the same re-pin; the test in
//! `storage_grant_tinybox_tests.rs` pins the shape.
//!
//! * `tinybox_boxes`: one document per box record, keyed by box id;
//! * `tinybox_jail_index`: one document per jail registry, the same JSON the
//!   registry used to keep in `index.json`.
//!
//! No streams, blobs or secrets: box contents live on the box's own disk.

use tinystoragedrivers::CollectionSpec;

use super::storage_grants::StorageGrant;

/// The registry id the grant is declared under.
pub(super) const MODULE_ID: &str = "tinybox";

/// Box records, one document per box.
pub(super) const BOX_COLLECTION: &str = "tinybox_boxes";
/// Jail registry indexes, one document per registry.
pub(super) const JAIL_INDEX_COLLECTION: &str = "tinybox_jail_index";

/// The grant the host serves the `tinybox` module under.
pub(super) fn grant() -> StorageGrant {
    StorageGrant {
        collections: vec![
            CollectionSpec::new(BOX_COLLECTION),
            CollectionSpec::new(JAIL_INDEX_COLLECTION),
        ],
        stream_prefixes: Vec::new(),
        blob_prefixes: Vec::new(),
        secret_prefixes: Vec::new(),
        writable: true,
    }
}

#[cfg(test)]
#[path = "storage_grant_tinybox_tests.rs"]
mod tests;
