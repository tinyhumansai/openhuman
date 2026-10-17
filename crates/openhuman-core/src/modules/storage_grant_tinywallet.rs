//! What the `tinywallet` module may touch through the storage bus.
//!
//! The names are the x402 ledger's own (`tinywallet_x402::ledger::persist` in
//! tinyhumansai/tinywallet, v0.22.0), restated here because a host never links
//! a module's implementation library: a grant is a reviewable build-time
//! decision, like the registry's digests. When the module's declaration
//! changes, this table changes in the same re-pin; the test in
//! `storage_grant_tinywallet_tests.rs` pins the shape.
//!
//! * `x402.payments`: the append-only payments stream. A payment that changes
//!   status is appended again with the same id; readers fold to the latest.
//! * `x402_budget`: one document, `holds`, with every outstanding budget hold.
//!   Reservations are compare-and-swap on it, so two processes sharing a
//!   backend cannot both spend the same budget. A hold expires after 15
//!   minutes unless the module renews it (every 5), so a process that dies
//!   mid-payment never locks budget forever.
//!
//! No blobs or secrets: the signing keys stay with the host's keyring and
//! reach the module only through its confidential calls.

use tinystoragedrivers::CollectionSpec;

use super::storage_grants::StorageGrant;

/// The registry id the grant is declared under.
pub(super) const MODULE_ID: &str = "tinywallet";

/// The payments stream.
pub(super) const PAYMENTS_STREAM: &str = "x402.payments";
/// The collection holding the budget-hold document.
pub(super) const BUDGET_COLLECTION: &str = "x402_budget";

/// The grant the host serves the `tinywallet` module under.
pub(super) fn grant() -> StorageGrant {
    StorageGrant {
        collections: vec![CollectionSpec::new(BUDGET_COLLECTION)],
        stream_prefixes: vec![PAYMENTS_STREAM.into()],
        blob_prefixes: Vec::new(),
        secret_prefixes: Vec::new(),
        writable: true,
    }
}

#[cfg(test)]
#[path = "storage_grant_tinywallet_tests.rs"]
mod tests;
