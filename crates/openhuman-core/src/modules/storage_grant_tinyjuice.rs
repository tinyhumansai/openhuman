//! What the `tinyjuice` module may touch through the storage bus.
//!
//! The prefix is the CCR durable tier's own (`tinyjuice_drivers::CCR_PREFIX` in
//! tinyhumansai/tinyjuice, v0.10.0), restated here because a host never links
//! a module's implementation library: a grant is a reviewable build-time
//! decision, like the registry's digests. When the module's declaration
//! changes, this table changes in the same re-pin; the test in
//! `storage_grant_tinyjuice_tests.rs` pins the shape.
//!
//! * `tinyjuice_ccr/<token>`: one expiring blob per compressed original, the
//!   content the model can ask to recover. The lifetime is the configured CCR
//!   TTL, set by the module on each write.
//!
//! No collections, streams or secrets.

use super::storage_grants::StorageGrant;

/// The registry id the grant is declared under.
pub(super) const MODULE_ID: &str = "tinyjuice";

/// Prefix of every CCR blob key.
pub(super) const CCR_BLOB_PREFIX: &str = "tinyjuice_ccr/";

/// The grant the host serves the `tinyjuice` module under.
pub(super) fn grant() -> StorageGrant {
    StorageGrant {
        collections: Vec::new(),
        stream_prefixes: Vec::new(),
        blob_prefixes: vec![CCR_BLOB_PREFIX.into()],
        secret_prefixes: Vec::new(),
        writable: true,
    }
}

#[cfg(test)]
#[path = "storage_grant_tinyjuice_tests.rs"]
mod tests;
