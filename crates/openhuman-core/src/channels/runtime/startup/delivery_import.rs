//! One-time, read-only import of the legacy delivery queue into the module's
//! storage.
//!
//! Before the storage contract the host kept TinyChannels' pending, failed and
//! terminal-result delivery records in `<workspace>/channels/channel-delivery.json`
//! and served them through persistence callbacks. The module now keeps them as
//! documents in its own scope of the host's storage (see
//! `modules::storage_grant_tinychannels`), so an install upgraded with required
//! deliveries still queued must not lose them.
//!
//! The legacy file is never modified or deleted. Every record is written with
//! `Precondition::Absent`, so a record the module has since written is never
//! overwritten, and a marker document (in a collection the module's grant does
//! not include) makes the import run once: a delivery completed afterwards is
//! not resurrected from the untouched file. An unreadable file writes no marker
//! and is retried at the next start.
//!
//! This persisted-data migration is independent of the TinyChannels v1/v2
//! protocol shim. Keep it until a release explicitly closes the workspace-file
//! migration window; removing it earlier could discard queued or failed
//! deliveries from an upgraded workspace.

use std::collections::BTreeMap;
use std::path::Path;

use crate::storage::{documents::Repo, Scope};
use serde_json::{json, Value};
use sha2::Digest;
use tinychannels_bus::module_delivery::{
    DeliveryQueueEntry, DeliveryTerminalResult, DELIVERY_PENDING_CAPACITY, DELIVERY_RECORD_BYTES,
    DELIVERY_RESULT_CAPACITY, DELIVERY_RESULT_TTL_SECONDS,
};
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition, StorageBackend};

const LOG_PREFIX: &str = "[channel-delivery-import]";
const MODULE_ID: &str = "tinychannels";
const LEGACY_FILE: &str = "channels/channel-delivery.json";
const MAX_LEGACY_BYTES: u64 = (DELIVERY_PENDING_CAPACITY * DELIVERY_RECORD_BYTES * 2) as u64
    + (DELIVERY_RESULT_CAPACITY * DELIVERY_RECORD_BYTES) as u64;

/// The module's delivery collections (declared by the module, restated in its
/// grant).
const PENDING: &str = "channels_delivery_pending";
const FAILED: &str = "channels_delivery_failed";
const RESULTS: &str = "channels_delivery_results";
/// Host-only: not part of the module's grant, so the module cannot see it.
const MARKER_COLLECTION: &str = "channels_host_import";

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct LegacyState {
    pending: BTreeMap<String, DeliveryQueueEntry>,
    failed: BTreeMap<String, DeliveryQueueEntry>,
    results: BTreeMap<String, DeliveryTerminalResult>,
}

/// How one import attempt ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// No storage backend is installed; nothing to write to.
    NoBackend,
    /// The marker says the import already finished.
    AlreadyDone,
    /// The claim could not be acquired before its wait budget expired.
    InProgress,
    /// There is no legacy file.
    NoSource,
    /// The legacy file was read and its records written.
    Imported { found: usize, written: usize },
}

/// Import the legacy delivery file under `workspace` into the module's scope of
/// the installed storage backend.
pub(super) async fn import_legacy_delivery(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
) -> anyhow::Result<Outcome> {
    let Some(backend) = crate::storage::installed() else {
        return Ok(Outcome::NoBackend);
    };
    let scope = crate::modules::storage_host::current_module_scope(ctx, MODULE_ID)
        .map_err(|error| anyhow::anyhow!("resolve the module scope: {:?}", error.kind()))?;
    import_into(&backend, &scope, workspace).await
}

async fn import_into(
    backend: &std::sync::Arc<dyn StorageBackend>,
    scope: &Scope,
    workspace: &Path,
) -> anyhow::Result<Outcome> {
    let path = workspace.join(LEGACY_FILE);
    let marker = marker_id(workspace);
    let markers = Repo::on(
        backend,
        scope,
        "storage::import",
        crate::storage::import_state::collection_specs,
    )?;
    let initial = backend
        .for_scope(scope)
        .map_err(|error| anyhow::anyhow!("open the module scope: {:?}", error.kind()))?;
    if crate::storage::import_state::host_import_done(&initial, MARKER_COLLECTION, &marker)
        .await
        .map_err(|error| anyhow::anyhow!("read the import state: {:?}", error.kind()))?
    {
        return Ok(Outcome::AlreadyDone);
    }
    let step = crate::storage::import_state::step_id(MARKER_COLLECTION, &marker);
    let claimed = crate::storage::import_state::with_claim(
        &markers,
        crate::storage::import_state::HOST_OWNER,
        &step,
        |condition| {
            let claimed_markers = markers.conditioned(condition)?;
            let claimed_backend = crate::storage::import_state::backend_with_condition(
                std::sync::Arc::clone(backend),
                condition.clone(),
            );
            let scoped = claimed_backend.for_scope(scope).map_err(|error| {
                anyhow::anyhow!("open claimed module scope: {:?}", error.kind())
            })?;
            crate::storage::block_on_anyhow(async move {
                if crate::storage::import_state::host_import_done(
                    claimed_markers.scoped(),
                    MARKER_COLLECTION,
                    &marker,
                )
                .await
                .map_err(|error| anyhow::anyhow!("read the import state: {:?}", error.kind()))?
                {
                    return Ok(Outcome::AlreadyDone);
                }
                let outcome = import_payload(&scoped, &path).await?;
                if matches!(outcome, Outcome::NoSource) {
                    return Ok(outcome);
                }
                crate::storage::import_state::host_import_mark(
                    claimed_markers.scoped(),
                    MARKER_COLLECTION,
                    &marker,
                )
                .await
                .map_err(|error| anyhow::anyhow!("write the import state: {:?}", error.kind()))?;
                Ok(outcome)
            })
        },
    )?;
    Ok(claimed.unwrap_or(Outcome::InProgress))
}

async fn import_payload(scoped: &crate::storage::Storage, path: &Path) -> anyhow::Result<Outcome> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Outcome::NoSource),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_LEGACY_BYTES,
        "legacy delivery file is oversized"
    );
    let legacy: LegacyState = serde_json::from_slice(&bytes)
        .map_err(|error| anyhow::anyhow!("legacy delivery file is unreadable: {error}"))?;
    let docs = scoped.documents();
    for name in [PENDING, FAILED, RESULTS] {
        docs.ensure_collection(&CollectionSpec::new(name))
            .await
            .map_err(|error| anyhow::anyhow!("declare {name}: {:?}", error.kind()))?;
    }
    let now = now_unix_ms();
    let ttl_ms = DELIVERY_RESULT_TTL_SECONDS.saturating_mul(1000);
    let mut records: Vec<(&str, String, Value)> = Vec::new();
    for (id, entry) in legacy.pending {
        if entry.id == id && entry.intent.idempotency_key == id {
            records.push((PENDING, id, serde_json::to_value(&entry)?));
        }
    }
    for (id, entry) in legacy.failed {
        if entry.id == id && entry.intent.idempotency_key == id {
            records.push((FAILED, id, serde_json::to_value(&entry)?));
        }
    }
    for (id, result) in legacy.results {
        let expires = result.completed_at_unix_ms.saturating_add(ttl_ms);
        if result.id != id || expires <= now {
            continue;
        }
        let doc = json!({
            "result": serde_json::to_value(&result)?,
            "completedAtUnixMs": result.completed_at_unix_ms,
            "expiresAtUnixMs": expires,
        });
        records.push((RESULTS, id, doc));
    }
    let found = records.len();
    let mut written = 0usize;
    for (collection, id, doc) in records {
        match docs.put(collection, &id, doc, Precondition::Absent).await {
            Ok(_) => written += 1,
            Err(error)
                if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) => {}
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "write {collection} record: {:?}",
                    error.kind()
                ));
            }
        }
    }
    tracing::info!("{LOG_PREFIX} imported found={found} written={written}");
    Ok(Outcome::Imported { found, written })
}

fn marker_id(workspace: &Path) -> String {
    let digest = sha2::Sha256::digest(workspace.as_os_str().as_encoded_bytes());
    format!("delivery@{}", &hex::encode(digest)[..16])
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
#[path = "delivery_import_tests.rs"]
mod tests;
