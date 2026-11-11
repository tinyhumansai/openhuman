//! Host-owned completion markers and expiring claims for one-time imports.
//!
//! Import progress is ordinary owner data in `storage.import_state`. The
//! collection is deliberately separate from Tiny Storage Drivers' schemas:
//! the host owns importer names, completion records, and claim policy.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tinystoragedrivers::{
    CollectionSpec, ErrorKind, ExpectedDocument, Filter, Precondition, ReadCheck, Storage,
    StorageBackend, StorageError, Versioned, WriteCondition, WriteOp,
};

use super::{documents::Repo, DocumentStoreExt};

/// How long a claim blocks other processes before it can be taken over.
pub const CLAIM_TTL_MS: i64 = 10 * 60 * 1000;

const COLLECTION: &str = "storage.import_state";
const LOG_PREFIX: &str = "[storage::import_state]";
const WAIT_POLL: Duration = Duration::from_millis(100);
const CAS_RETRIES: usize = 32;

/// The legacy marker collections whose completion rows are moved at boot.
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

/// Declares the host-owned import progress collection.
#[must_use]
pub fn collection_specs() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(COLLECTION)]
}

/// The stable step key for a marker in an older collection.
#[must_use]
pub fn step_id(kind: &str, id: &str) -> String {
    format!("{kind}/{id}")
}

/// The owner name used by module importers.
pub const HOST_OWNER: &str = "host";

/// Returns a backend whose scope handles carry `condition` on every write,
/// except collection declarations, which every driver must accept from an
/// importer (see `claim_bound`). Calling `for_scope_conditioned` on the
/// wrapped backend composes the condition with any profile-level condition
/// before opening the target.
pub(crate) fn backend_with_condition(
    backend: std::sync::Arc<dyn StorageBackend>,
    condition: WriteCondition,
) -> std::sync::Arc<dyn StorageBackend> {
    std::sync::Arc::new(ClaimBoundBackend { backend, condition })
}

struct ClaimBoundBackend {
    backend: std::sync::Arc<dyn StorageBackend>,
    condition: WriteCondition,
}

impl std::fmt::Debug for ClaimBoundBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaimBoundBackend")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl StorageBackend for ClaimBoundBackend {
    fn capabilities(&self) -> tinystoragedrivers::Capabilities {
        self.backend.capabilities()
    }

    fn for_scope(&self, scope: &tinystoragedrivers::Scope) -> Result<Storage, StorageError> {
        super::claim_bound::claim_bound(self.backend.as_ref(), scope, &self.condition)
    }

    fn for_scope_conditioned(
        &self,
        scope: &tinystoragedrivers::Scope,
        condition: &WriteCondition,
    ) -> Result<Storage, StorageError> {
        let condition = self
            .condition
            .clone()
            .and(condition.clone())
            .map_err(|error| StorageError::invalid_input(error.to_string()))?;
        super::claim_bound::claim_bound(self.backend.as_ref(), scope, &condition)
    }
}

fn record_key(owner: &str, step: &str, kind: &str) -> String {
    // Hex encoding keeps this key injective even when either source component
    // contains separators used by the document-id grammar.
    format!(
        "{kind}/{}/{}",
        hex::encode(owner.as_bytes()),
        hex::encode(step.as_bytes())
    )
}

async fn ensure(scoped: &Storage) -> Result<(), StorageError> {
    for spec in collection_specs() {
        scoped.documents().ensure_collection(&spec).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Completion {
    record_type: String,
    owner: String,
    step: String,
    completed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claim {
    record_type: String,
    owner: String,
    step: String,
    holder: Option<String>,
    epoch: u64,
    valid_until_ms: i64,
}

fn decode<T: for<'de> Deserialize<'de>>(
    row: Versioned<serde_json::Value>,
    expected_type: &str,
    expected_owner: &str,
    expected_step: &str,
) -> Result<T, StorageError> {
    let record_type = row
        .doc
        .get("record_type")
        .and_then(serde_json::Value::as_str);
    let owner = row.doc.get("owner").and_then(serde_json::Value::as_str);
    let step = row.doc.get("step").and_then(serde_json::Value::as_str);
    if record_type != Some(expected_type)
        || owner != Some(expected_owner)
        || step != Some(expected_step)
    {
        return Err(StorageError::serialization(
            "import state key does not match its owner and step",
        ));
    }
    serde_json::from_value(row.doc)
        .map_err(|error| StorageError::serialization(format!("invalid import state: {error}")))
}

/// Checks whether the import `step` for `owner` is complete in `scoped`.
pub async fn done(scoped: &Storage, owner: &str, step: &str) -> Result<bool, StorageError> {
    ensure(scoped).await?;
    let id = record_key(owner, step, "done");
    let Some(row) = scoped.documents().get(COLLECTION, &id).await? else {
        return Ok(false);
    };
    let _: Completion = decode(row, "completion", owner, step)?;
    Ok(true)
}

/// Records completion of an import step. Existing completion is immutable.
pub async fn mark_done(scoped: &Storage, owner: &str, step: &str) -> Result<(), StorageError> {
    mark_done_at(scoped, owner, step, chrono::Utc::now().timestamp_millis()).await
}

async fn mark_done_at(
    scoped: &Storage,
    owner: &str,
    step: &str,
    completed_at_ms: i64,
) -> Result<(), StorageError> {
    ensure(scoped).await?;
    let id = record_key(owner, step, "done");
    let body = serde_json::to_value(Completion {
        record_type: "completion".to_owned(),
        owner: owner.to_owned(),
        step: step.to_owned(),
        completed_at_ms,
    })
    .map_err(|error| StorageError::serialization(error.to_string()))?;
    for _ in 0..CAS_RETRIES {
        if let Some(current) = scoped.documents().get(COLLECTION, &id).await? {
            let _: Completion = decode(current, "completion", owner, step)?;
            return Ok(());
        }
        match scoped
            .documents()
            .put(COLLECTION, &id, body.clone(), Precondition::Absent)
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::Conflict => continue,
            Err(error) => return Err(error),
        }
    }
    Err(StorageError::conflict(
        "import completion kept changing during its retry budget",
    ))
}

/// Whether the module importer has completed a legacy marker.
pub async fn host_import_done(
    scoped: &Storage,
    kind: &str,
    id: &str,
) -> Result<bool, StorageError> {
    done(scoped, HOST_OWNER, &step_id(kind, id)).await
}

/// Records completion of a module import marker.
pub async fn host_import_mark(scoped: &Storage, kind: &str, id: &str) -> Result<(), StorageError> {
    mark_done(scoped, HOST_OWNER, &step_id(kind, id)).await
}

/// Synchronous completion lookup for existing synchronous import APIs.
pub fn is_done(repo: &Repo, owner: &str, step: &str) -> Result<bool> {
    let scoped = repo.scoped().clone();
    let (owner, step) = (owner.to_owned(), step.to_owned());
    repo.run(move |_| async move { done(&scoped, &owner, &step).await })
}

/// Synchronous completion write for existing synchronous import APIs.
pub fn set_done(repo: &Repo, owner: &str, step: &str) -> Result<()> {
    let scoped = repo.scoped().clone();
    let (owner, step) = (owner.to_owned(), step.to_owned());
    repo.run(move |_| async move { mark_done(&scoped, &owner, &step).await })
}

/// Runs `body` while holding a claim for this importer step.
///
/// A competing live claim is polled until release or expiry, bounded by the
/// claim TTL. The body must re-check completion after acquiring the claim.
pub fn with_claim<T>(
    repo: &Repo,
    owner: &str,
    step: &str,
    body: impl FnOnce(&WriteCondition) -> Result<T>,
) -> Result<Option<T>> {
    with_claim_ttl(repo, owner, step, CLAIM_TTL_MS, body)
}

/// Runs `body` under a claim like [`with_claim`], but returns `Ok(None)` at
/// once when another holder's claim is live instead of polling for up to the
/// claim TTL. For callers on an async path that retry at their next call.
pub fn try_claim<T>(
    repo: &Repo,
    owner: &str,
    step: &str,
    body: impl FnOnce(&WriteCondition) -> Result<T>,
) -> Result<Option<T>> {
    claim_waiting(repo, owner, step, CLAIM_TTL_MS, Duration::ZERO, body)
}

fn with_claim_ttl<T>(
    repo: &Repo,
    owner: &str,
    step: &str,
    ttl_ms: i64,
    body: impl FnOnce(&WriteCondition) -> Result<T>,
) -> Result<Option<T>> {
    let max_wait = Duration::from_millis(CLAIM_TTL_MS.max(0) as u64);
    claim_waiting(repo, owner, step, ttl_ms, max_wait, body)
}

fn claim_waiting<T>(
    repo: &Repo,
    owner: &str,
    step: &str,
    ttl_ms: i64,
    max_wait: Duration,
    body: impl FnOnce(&WriteCondition) -> Result<T>,
) -> Result<Option<T>> {
    let started = Instant::now();
    let mut body = Some(body);
    loop {
        let now = chrono::Utc::now().timestamp_millis();
        if let Some(token) = acquire(repo, owner, step, now, ttl_ms)? {
            let condition = token.write_condition()?;
            let (stop_tx, stop_rx) = std::sync::mpsc::channel();
            let healthy = std::sync::Arc::new(AtomicBool::new(true));
            let heartbeat = start_heartbeat(
                repo.clone(),
                token.clone(),
                ttl_ms,
                stop_rx,
                std::sync::Arc::clone(&healthy),
            );
            let result = body.take().expect("import body runs once")(&condition);
            let _ = stop_tx.send(());
            let _ = heartbeat.join();
            release(repo, &token)?;
            if !healthy.load(Ordering::Acquire) {
                return Err(anyhow::anyhow!(
                    "import claim was lost before its operation completed"
                ));
            }
            return result.map(Some);
        }
        if started.elapsed() >= max_wait {
            return Ok(None);
        }
        std::thread::sleep(WAIT_POLL);
    }
}

/// One bounded claim attempt with an explicit clock and TTL, primarily for
/// deterministic concurrency tests.
pub fn with_claim_at<T>(
    repo: &Repo,
    owner: &str,
    step: &str,
    now_ms: i64,
    ttl_ms: i64,
    body: impl FnOnce(&WriteCondition) -> Result<T>,
) -> Result<Option<T>> {
    let Some(token) = acquire(repo, owner, step, now_ms, ttl_ms)? else {
        return Ok(None);
    };
    let condition = token.write_condition()?;
    let result = body(&condition);
    release(repo, &token)?;
    result.map(Some)
}

#[derive(Clone)]
struct ClaimToken {
    owner: String,
    step: String,
    scope: String,
    holder: String,
    epoch: u64,
    ttl_ms: i64,
    clock_started: Instant,
    clock_start_ms: i64,
}

impl ClaimToken {
    fn write_condition(&self) -> Result<WriteCondition> {
        let scope = tinystoragedrivers::Scope::new(&self.scope)?;
        let filter = Filter::eq("holder", self.holder.clone())
            .and(Filter::eq("epoch", serde_json::json!(self.epoch)));
        Ok(WriteCondition::new(
            scope,
            COLLECTION,
            record_key(&self.owner, &self.step, "claim"),
            filter,
        )?
        .with_valid_until_field("valid_until_ms")?)
    }
}

fn acquire(
    repo: &Repo,
    owner: &str,
    step: &str,
    now_ms: i64,
    ttl_ms: i64,
) -> Result<Option<ClaimToken>> {
    if ttl_ms <= 0 {
        return Err(anyhow::anyhow!("import claim TTL must be positive"));
    }
    let holder = format!("pid-{}-{}", std::process::id(), uuid::Uuid::new_v4());
    let scope = repo
        .origin_scope()
        .ok_or_else(|| anyhow::anyhow!("import claims require an opened backend scope"))?
        .to_owned();
    let (owner_owned, step_owned, holder_owned) =
        (owner.to_owned(), step.to_owned(), holder.clone());
    let scoped = repo.scoped().clone();
    let outcome = repo.run(move |docs| async move {
        ensure(&scoped).await?;
        let id = record_key(&owner_owned, &step_owned, "claim");
        for _ in 0..CAS_RETRIES {
            let current = docs.get(COLLECTION, &id).await?;
            let (expected, precondition, epoch) = match current {
                Some(row) => {
                    let claim: Claim = decode(row.clone(), "claim", &owner_owned, &step_owned)?;
                    if claim.holder.is_some() && claim.valid_until_ms > now_ms {
                        return Ok(None);
                    }
                    let epoch = claim
                        .epoch
                        .checked_add(1)
                        .ok_or_else(|| StorageError::backend("import claim epoch exhausted"))?;
                    (
                        ExpectedDocument::Version(row.version),
                        Precondition::Version(row.version),
                        epoch,
                    )
                }
                None => (ExpectedDocument::Absent, Precondition::Absent, 1),
            };
            let valid_until_ms = now_ms.saturating_add(ttl_ms);
            let body = serde_json::to_value(Claim {
                record_type: "claim".to_owned(),
                owner: owner_owned.clone(),
                step: step_owned.clone(),
                holder: Some(holder_owned.clone()),
                epoch,
                valid_until_ms,
            })
            .map_err(|error| StorageError::serialization(error.to_string()))?;
            let check = ReadCheck {
                collection: COLLECTION.to_owned(),
                id: id.clone(),
                expected,
            };
            let write = WriteOp::Put {
                collection: COLLECTION.to_owned(),
                id: id.clone(),
                doc: body,
                precondition,
            };
            match docs.atomic_batch_checked(vec![check], vec![write]).await {
                Ok(_) => return Ok(Some(epoch)),
                Err(error) if error.kind() == ErrorKind::Conflict => continue,
                Err(error) => return Err(error),
            }
        }
        Err(StorageError::conflict(
            "import claim kept changing during its retry budget",
        ))
    })?;
    Ok(outcome.map(|epoch| ClaimToken {
        owner: owner.to_owned(),
        step: step.to_owned(),
        scope,
        holder,
        epoch,
        ttl_ms,
        clock_started: Instant::now(),
        clock_start_ms: now_ms,
    }))
}

fn start_heartbeat(
    repo: Repo,
    token: ClaimToken,
    ttl_ms: i64,
    stop: std::sync::mpsc::Receiver<()>,
    healthy: std::sync::Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let interval = Duration::from_millis((ttl_ms / 3).max(1) as u64);
    std::thread::spawn(move || {
        while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(interval) {
            let now_ms = token
                .clock_start_ms
                .saturating_add(token.clock_started.elapsed().as_millis() as i64);
            match renew(&repo, &token, now_ms) {
                Ok(true) => {}
                Ok(false) | Err(_) => {
                    healthy.store(false, Ordering::Release);
                    tracing::warn!(target: "openhuman::storage_import", "{LOG_PREFIX} could not renew the import claim");
                    break;
                }
            }
        }
    })
}

fn renew(repo: &Repo, token: &ClaimToken, now_ms: i64) -> Result<bool> {
    let scoped = repo.scoped().clone();
    let token = token.clone();
    repo.run(move |docs| async move {
        ensure(&scoped).await?;
        let id = record_key(&token.owner, &token.step, "claim");
        let Some(row) = docs.get(COLLECTION, &id).await? else {
            return Ok(false);
        };
        let mut claim: Claim = decode(row.clone(), "claim", &token.owner, &token.step)?;
        if claim.holder.as_deref() != Some(&token.holder)
            || claim.epoch != token.epoch
            || claim.valid_until_ms <= now_ms
        {
            return Ok(false);
        }
        claim.valid_until_ms = now_ms.saturating_add(token.ttl_ms);
        let body = serde_json::to_value(claim)
            .map_err(|error| StorageError::serialization(error.to_string()))?;
        let check = ReadCheck {
            collection: COLLECTION.to_owned(),
            id: id.clone(),
            expected: ExpectedDocument::Version(row.version),
        };
        let write = WriteOp::Put {
            collection: COLLECTION.to_owned(),
            id,
            doc: body,
            precondition: Precondition::Version(row.version),
        };
        match docs.atomic_batch_checked(vec![check], vec![write]).await {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == ErrorKind::Conflict => Ok(false),
            Err(error) => Err(error),
        }
    })
}

fn release(repo: &Repo, token: &ClaimToken) -> Result<()> {
    let scoped = repo.scoped().clone();
    let token = token.clone();
    repo.run(move |docs| async move {
        ensure(&scoped).await?;
        let id = record_key(&token.owner, &token.step, "claim");
        let Some(row) = docs.get(COLLECTION, &id).await? else {
            return Ok(());
        };
        let mut claim: Claim = decode(row.clone(), "claim", &token.owner, &token.step)?;
        if claim.holder.as_deref() != Some(&token.holder) || claim.epoch != token.epoch {
            return Ok(());
        }
        claim.holder = None;
        claim.valid_until_ms = 0;
        let body = serde_json::to_value(claim)
            .map_err(|error| StorageError::serialization(error.to_string()))?;
        let check = ReadCheck {
            collection: COLLECTION.to_owned(),
            id: id.clone(),
            expected: ExpectedDocument::Version(row.version),
        };
        let write = WriteOp::Put {
            collection: COLLECTION.to_owned(),
            id,
            doc: body,
            precondition: Precondition::Version(row.version),
        };
        match docs.atomic_batch_checked(vec![check], vec![write]).await {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::Conflict => Ok(()),
            Err(error) => Err(error),
        }
    })
}

/// Migrates older import marker documents to host-owned completion rows.
pub async fn migrate_legacy_markers(scoped: &Storage) -> Result<usize, StorageError> {
    let docs = scoped.documents();
    if !docs
        .capabilities()
        .contains(tinystoragedrivers::Capability::CollectionEnumeration)
    {
        return Ok(0);
    }
    let present = docs.collections("").await?;
    let mut moved = 0;
    for (collection, owner) in LEGACY_MARKERS {
        if !present.iter().any(|name| name == collection) {
            continue;
        }
        ensure(scoped).await?;
        for marker in docs
            .query_all(collection, &tinystoragedrivers::Query::all())
            .await?
        {
            if marker.id.ends_with("#claim") {
                continue;
            }
            let completed_at_ms = if marker.meta.created_ms > 0 {
                marker.meta.created_ms
            } else {
                chrono::Utc::now().timestamp_millis()
            };
            mark_done_at(
                scoped,
                owner,
                &step_id(collection, &marker.id),
                completed_at_ms,
            )
            .await?;
            moved += 1;
        }
        docs.drop_collection(collection).await?;
        tracing::info!(target: "openhuman::storage", collection, owner, "{LOG_PREFIX} legacy import markers moved to host-owned state");
    }
    Ok(moved)
}

#[cfg(test)]
#[path = "import_state_tests.rs"]
mod tests;
