//! Where a one-time legacy importer records that it finished, and the claim
//! that keeps two processes from importing at once.
//!
//! Both are tsd primitives, not host collections:
//!
//! - **Completion** is a `tsd.migrations` entry ([`Migrations::mark_done`]) in
//!   the scope the importer writes to, keyed `(owner, step)`. This replaces
//!   the per-domain marker collections (`storage_import`, `threads_import`,
//!   `subagent_sessions_import`, `composio_state_import` and the module
//!   `*_host_import` ones). A marker row an older build wrote is honoured by
//!   [`migrate_legacy_markers`], which turns each into the same entry at boot,
//!   so an installed profile is never imported twice.
//! - **The claim** is a [`Leases`] lease on `(owner, step)` in `tsd.leases`
//!   (schema-v2 section 5). The process that wins it copies the legacy
//!   records; the others wait it out, then re-check completion. The owner
//!   renews the lease while it works, and a lease that lapses (its owner
//!   crashed) is taken over by the next acquirer, so a dead process never
//!   blocks an import.
//!
//! A step is named `<kind>/<id>`: `kind` is the old marker collection name
//! (`storage_import`) and `id` the old marker id (`approval@<workspace
//! digest>`), so both the new check and the migrated entry derive from the
//! same pair.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tinystoragedrivers::schema::Migrations;
use tinystoragedrivers::{
    Capability, DocumentStore, DocumentStoreExt, Leases, Query, ScopedStorage, StorageError,
};

use super::documents::Repo;

/// How long a claim blocks other processes before it counts as abandoned.
pub const CLAIM_TTL_MS: i64 = 10 * 60 * 1000;

/// The `tsd.leases` owner of import claims.
const CLAIM_OWNER: &str = "import";

const LOG_PREFIX: &str = "[storage::import_state]";

/// How often a waiting process re-tries a claim another process holds.
const WAIT_POLL: Duration = Duration::from_millis(100);

/// The legacy marker collections and the `tsd.migrations` owner each moves
/// under. A marker lived in the scope its importer wrote to, so every scope
/// is swept.
pub const LEGACY_MARKERS: &[(&str, &str)] = &[
    ("storage_import", "storage"),
    ("threads_import", "threads"),
    ("subagent_sessions_import", "subagents"),
    ("composio_state_import", "composio"),
    ("channels_host_import", "host"),
    ("tinybox_host_import", "host"),
    ("tinyconnectors_host_import", "host"),
    ("tinywallet_host_import", "host"),
    ("tinyjuice_host_import", "host"),
    ("mcp_host_import", "host"),
];

/// The step name of the importer that wrote marker `id` into `kind`.
#[must_use]
pub fn step_id(kind: &str, id: &str) -> String {
    format!("{kind}/{id}")
}

/// Declares the primitives an import state needs, for a backend that did not
/// go through the boot `prepare` (a test's, an embedder's).
async fn declare(docs: &Arc<dyn DocumentStore>) -> Result<(), StorageError> {
    for spec in tinystoragedrivers::primitives::collection_specs() {
        docs.ensure_collection(&spec).await?;
    }
    Ok(())
}

/// Whether importer `step` of `owner` finished in `scoped`'s scope.
///
/// # Errors
///
/// A storage error.
pub async fn done(scoped: &ScopedStorage, owner: &str, step: &str) -> Result<bool, StorageError> {
    declare(scoped.documents()).await?;
    Migrations::new(scoped).done(owner, step).await
}

/// Records importer `step` of `owner` as finished in `scoped`'s scope.
///
/// # Errors
///
/// A storage error.
pub async fn mark_done(
    scoped: &ScopedStorage,
    owner: &str,
    step: &str,
) -> Result<(), StorageError> {
    declare(scoped.documents()).await?;
    Migrations::new(scoped).mark_done(owner, step).await
}

/// The `tsd.migrations` owner of the module importers, which run in a module's
/// scope on behalf of the host (`kind` is the old marker collection name).
pub const HOST_OWNER: &str = "host";

/// Whether the module importer `kind` already finished for marker `id` in
/// `scoped`'s (module) scope.
///
/// # Errors
///
/// A storage error.
pub async fn host_import_done(
    scoped: &ScopedStorage,
    kind: &str,
    id: &str,
) -> Result<bool, StorageError> {
    done(scoped, HOST_OWNER, &step_id(kind, id)).await
}

/// Records the module importer `kind` as finished for marker `id`.
///
/// # Errors
///
/// A storage error.
pub async fn host_import_mark(
    scoped: &ScopedStorage,
    kind: &str,
    id: &str,
) -> Result<(), StorageError> {
    mark_done(scoped, HOST_OWNER, &step_id(kind, id)).await
}

/// [`done`] from synchronous code, over the scope `repo` was opened on.
///
/// # Errors
///
/// A storage error.
pub fn is_done(repo: &Repo, owner: &str, step: &str) -> Result<bool> {
    let scoped = repo.scoped().clone();
    let (owner, step) = (owner.to_owned(), step.to_owned());
    repo.run(move |_| async move { done(&scoped, &owner, &step).await })
}

/// [`mark_done`] from synchronous code.
///
/// # Errors
///
/// A storage error.
pub fn set_done(repo: &Repo, owner: &str, step: &str) -> Result<()> {
    let scoped = repo.scoped().clone();
    let (owner, step) = (owner.to_owned(), step.to_owned());
    repo.run(move |_| async move { mark_done(&scoped, &owner, &step).await })
}

/// Runs `body` while holding the claim for `(owner, step)` in `repo`'s scope.
///
/// A live claim held by another process is waited out (up to
/// [`CLAIM_TTL_MS`]); once it is free `body` runs, and it must re-check its
/// completion because the winner has usually finished by then. Returns `None`
/// only when the claim stays held for the whole wait. The claim is renewed
/// while `body` runs, so a long import is not mistaken for a crashed one.
///
/// # Errors
///
/// A storage error while claiming, or whatever `body` returns. The claim is
/// released either way.
pub fn with_claim<T>(
    repo: &Repo,
    owner: &str,
    step: &str,
    body: impl FnOnce() -> Result<T>,
) -> Result<Option<T>> {
    // `with_claim_at` only runs the closure once it holds the claim.
    let mut body = Some(body);
    let started = Instant::now();
    loop {
        let now = chrono::Utc::now().timestamp_millis();
        if let Some(ran) = with_claim_at(repo, owner, step, now, CLAIM_TTL_MS, || {
            (body.take().expect("import body runs once"))()
        })? {
            return Ok(Some(ran));
        }
        if started.elapsed().as_millis() as i64 >= CLAIM_TTL_MS {
            return Ok(None);
        }
        std::thread::sleep(WAIT_POLL);
    }
}

/// One attempt of [`with_claim`] with an explicit clock and lease length, for
/// tests: returns `None` without running `body` or waiting when another live
/// process holds the claim.
pub fn with_claim_at<T>(
    repo: &Repo,
    owner: &str,
    step: &str,
    now_ms: i64,
    ttl_ms: i64,
    body: impl FnOnce() -> Result<T>,
) -> Result<Option<T>> {
    let started = Instant::now();
    let clock: tinystoragedrivers::Clock = Arc::new(move || {
        (now_ms.max(0) as u64).saturating_add(started.elapsed().as_millis() as u64)
    });
    // A holder name unique to this attempt, so two threads of one process do
    // not both "re-acquire" the same live lease.
    let holder = format!("pid-{}-{}", std::process::id(), uuid::Uuid::new_v4());
    let (claim_owner, claim_key) = (owner.to_owned(), step.to_owned());
    let acquired = {
        let (clock, holder) = (clock.clone(), holder.clone());
        let (owner, key) = (claim_owner.clone(), claim_key.clone());
        repo.run(move |docs| async move {
            declare(&docs).await?;
            Leases::new(&docs)
                .with_clock(clock)
                .acquire(CLAIM_OWNER, &lease_key(&owner, &key), &holder, ttl_ms)
                .await
        })?
    };
    let Some(held) = acquired else {
        tracing::debug!(
            target: "openhuman::storage_import",
            owner,
            step,
            "{LOG_PREFIX} another process holds the import claim"
        );
        return Ok(None);
    };
    let epoch = held.epoch;
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let heartbeat = {
        let (repo, clock, holder) = (repo.clone(), clock.clone(), holder.clone());
        let key = lease_key(&claim_owner, &claim_key);
        let interval = Duration::from_millis((ttl_ms / 3).max(1) as u64);
        // A bare OS thread on purpose (baselined in the SaaS ambient ratchet):
        // the heartbeat reads no ambient state, only the `repo` handed to it,
        // and is joined below before this returns. `with_claim` is synchronous
        // and may run outside a tokio runtime, so `DispatchCtx::spawn_blocking`
        // (which needs `Handle::current()`) is not available here.
        std::thread::spawn(move || {
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                stop_rx.recv_timeout(interval)
            {
                let (clock, holder, key) = (clock.clone(), holder.clone(), key.clone());
                let renewed = repo.run(move |docs| async move {
                    Leases::new(&docs)
                        .with_clock(clock)
                        .renew(CLAIM_OWNER, &key, &holder, epoch, ttl_ms)
                        .await
                });
                match renewed {
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => {
                        tracing::warn!(
                            target: "openhuman::storage_import",
                            "{LOG_PREFIX} could not renew the import claim"
                        );
                        break;
                    }
                }
            }
        })
    };
    let result = body();
    let _ = stop_tx.send(());
    let _ = heartbeat.join();
    let released = {
        let key = lease_key(&claim_owner, &claim_key);
        repo.run(move |docs| async move {
            Leases::new(&docs)
                .with_clock(clock)
                .release(CLAIM_OWNER, &key, &holder, epoch)
                .await
        })
    };
    if let Err(error) = released {
        // The lease expires on its own; releasing is an optimisation.
        tracing::warn!(
            target: "openhuman::storage_import",
            error = %format!("{error:#}"),
            "{LOG_PREFIX} could not release the import claim; it expires on its own"
        );
    }
    result.map(Some)
}

fn lease_key(owner: &str, step: &str) -> String {
    format!("{owner}/{step}")
}

/// Turns the marker rows older builds wrote ([`LEGACY_MARKERS`]) in `scoped`'s
/// scope into `tsd.migrations` entries and removes the old collections, so an
/// already imported profile is not imported again. Returns the markers moved.
///
/// Idempotent: a marker already moved is simply marked again, and a collection
/// that is gone is skipped. A claim row (`<id>#claim`) is dropped with its
/// collection.
///
/// # Errors
///
/// A storage error; the markers not yet moved stay in place for the next boot.
pub async fn migrate_legacy_markers(scoped: &ScopedStorage) -> Result<usize, StorageError> {
    let docs = scoped.documents();
    if !docs
        .capabilities()
        .contains(Capability::CollectionEnumeration)
    {
        return Ok(0);
    }
    let present = docs.collections("").await?;
    let mut moved = 0;
    for (collection, owner) in LEGACY_MARKERS {
        if !present.iter().any(|name| name == collection) {
            continue;
        }
        declare(docs).await?;
        for marker in docs.query_all(collection, &Query::all()).await? {
            if marker.id.ends_with("#claim") {
                continue;
            }
            mark_done(scoped, owner, &step_id(collection, &marker.id)).await?;
            moved += 1;
        }
        docs.drop_collection(collection).await?;
        tracing::info!(
            target: "openhuman::storage",
            collection,
            owner,
            "{LOG_PREFIX} legacy import markers moved to tsd.migrations"
        );
    }
    Ok(moved)
}

#[cfg(test)]
#[path = "import_state_tests.rs"]
mod tests;
