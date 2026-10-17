//! The host's own boot-time importers: the moves a document step cannot make
//! because the target is another owner's collection.
//!
//! A [`Step`](tinystoragedrivers::schema::Step) may only write its own
//! owner's items (schema-v2 section 4), so v1 data whose v2 home is a tsd
//! primitive (`tsd.kv`, `tsd.counters`) is moved here, after
//! `Registry::migrate`, the same way the TinyAgents owners' post-migrate
//! importers are. Each is idempotent and drops its v1 collection once moved,
//! so a finished move costs one collection listing.

use tinystoragedrivers::{ScopedStorage, StorageError};

/// Runs every host importer over `scoped`. Returns `(importer, error)` for each
/// one that failed; the others still ran, and a failed one is retried at the
/// next boot.
pub(super) async fn run(scoped: &ScopedStorage) -> Vec<(&'static str, StorageError)> {
    let mut failed = Vec::new();
    if let Err(error) = crate::integrations::composio::migrate_legacy_state(scoped).await {
        failed.push(("composio_state_to_kv", error));
    }
    failed
}
