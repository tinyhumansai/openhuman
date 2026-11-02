//! What the one-time legacy importers (`storage_import_*`) share.
//!
//! Each importer moves a module's pre-storage files into the module's own
//! scope of the installed backend, once per source directory, never touching
//! the files. The shared parts are opening that scope, the host-only marker
//! that records a finished import, and a write that never replaces a record
//! the module has since written.

use std::path::Path;
use std::sync::Arc;

use crate::storage::{DocumentStore, Precondition, Storage};
use anyhow::{anyhow, Result};
use serde_json::Value;
use sha2::Digest;
use tinystoragedrivers::ErrorKind;
use tinystoragedrivers_types::WriteOptions;

/// Storage and profile admission captured from the import caller's context.
/// Keeping the profile alive prevents eviction until all writes using the
/// captured lease fence have completed.
pub(super) struct CapturedModuleStorage {
    pub(super) backend: Arc<dyn crate::storage::StorageBackend>,
    pub(super) scope: crate::storage::Scope,
    pub(super) _profile: Option<Arc<crate::profiles::Profile>>,
}

fn capture_module_storage(
    ctx: &crate::core::runtime::DispatchCtx,
    module_id: &str,
) -> Result<Option<CapturedModuleStorage>> {
    let Some(installed) = crate::storage::installed() else {
        return Ok(None);
    };
    let scope = super::storage_host::current_module_scope(ctx, module_id)
        .map_err(|error| anyhow!("resolve the module scope: {:?}", error.kind()))?;
    let tenant = ctx
        .tenant()
        .map_err(|error| anyhow!("resolve the import caller's tenant: {error}"))?;
    let Some(profile_id) = tenant.profile else {
        return Ok(Some(CapturedModuleStorage {
            backend: installed,
            scope,
            _profile: None,
        }));
    };

    let caller_context = ctx
        .resolved()
        .ok_or_else(|| anyhow!("profile import has no admitted caller context"))?;
    let host = crate::profiles::host::host()
        .ok_or_else(|| anyhow!("profile import has no active profile host"))?;
    let id = crate::profiles::ProfileId::parse(&profile_id)
        .map_err(|error| anyhow!("invalid caller profile id: {error}"))?;
    Ok(Some(capture_profile_module_storage(
        &host,
        &id,
        &profile_id,
        &caller_context,
        scope,
    )?))
}

pub(super) fn capture_profile_module_storage(
    host: &crate::profiles::ProfileHost,
    id: &crate::profiles::ProfileId,
    profile_id: &str,
    caller_context: &Arc<crate::core::runtime::CoreContext>,
    scope: crate::storage::Scope,
) -> Result<CapturedModuleStorage> {
    let profile = host
        .get(id)
        .ok_or_else(|| anyhow!("caller profile is no longer open"))?;
    require_profile_context(profile.context(), caller_context, profile_id)?;
    let backend = host
        .storage_backend_for(&profile)
        .map_err(|error| {
            anyhow!(
                "capture the caller profile's storage fence: {:?}",
                error.kind()
            )
        })?
        .ok_or_else(|| anyhow!("profile storage is unavailable for this import"))?;
    Ok(CapturedModuleStorage {
        backend,
        scope,
        _profile: Some(profile),
    })
}

/// Rejects work from an old context after a profile has been reopened under a
/// different lease. Callers must capture the backend from this same profile
/// before starting a claim, then retain the profile for the whole operation.
pub(super) fn require_profile_context(
    admitted: &Arc<crate::core::runtime::CoreContext>,
    caller: &Arc<crate::core::runtime::CoreContext>,
    profile_id: &str,
) -> Result<()> {
    if admitted.profile() == Some(profile_id)
        && caller.profile() == Some(profile_id)
        && admitted.shares_profile_admission(caller)
    {
        Ok(())
    } else {
        Err(anyhow!(
            "caller context is not the currently admitted context for profile {profile_id}"
        ))
    }
}

/// The module's scope of the installed backend, or `None` when the process
/// runs without a storage backend (the module is then not using this storage).
pub(super) fn module_scope(
    ctx: &crate::core::runtime::DispatchCtx,
    module_id: &str,
) -> Result<Option<Storage>> {
    let Some(captured) = capture_module_storage(ctx, module_id)? else {
        return Ok(None);
    };
    let scoped = captured
        .backend
        .for_scope(&captured.scope)
        .map_err(|error| anyhow!("open the module scope: {:?}", error.kind()))?;
    Ok(Some(scoped))
}

/// Runs one module import under a durable host claim. The body receives a
/// storage handle bound to the same claim condition and must use it for both
/// payload writes and the completion marker. A lost claim therefore prevents
/// late payload or completion writes.
///
/// `Ok(None)` means no storage backend is installed. When another holder's
/// claim is live this returns [`ImportClaimHeld`] at once instead of waiting, so an
/// import on the compaction path never blocks a runtime worker (or the
/// once-per-process import lock) for the claim TTL. The error keeps the import
/// unsettled and makes callers that gate on it (the x402 ledger) refuse for
/// this call; it is retried at the next one.
pub(super) fn with_module_claim<T>(
    ctx: &crate::core::runtime::DispatchCtx,
    module_id: &str,
    marker_kind: &str,
    marker_id: &str,
    body: impl FnOnce(Storage) -> Result<T> + Send + 'static,
) -> Result<Option<T>>
where
    T: Send + 'static,
{
    let Some(captured) = capture_module_storage(ctx, module_id)? else {
        return Ok(None);
    };
    let CapturedModuleStorage {
        backend,
        scope,
        _profile: profile,
    } = captured;
    let repo = crate::storage::documents::Repo::on(
        &backend,
        &scope,
        "storage_import",
        crate::storage::import_state::collection_specs,
    )?;
    let claim_backend = Arc::clone(&backend);
    let claim_scope = scope.clone();
    let claimed = crate::storage::import_state::try_claim(
        &repo,
        crate::storage::import_state::HOST_OWNER,
        &crate::storage::import_state::step_id(marker_kind, marker_id),
        move |condition| {
            // Keep the admitted profile context alive across claim, payload,
            // and completion writes; the backend itself holds its exact
            // captured fence and never performs a later registry lookup.
            let _profile = profile;
            let scoped_backend = crate::storage::import_state::backend_with_condition(
                claim_backend,
                condition.clone(),
            );
            let scoped = scoped_backend
                .for_scope(&claim_scope)
                .map_err(|error| anyhow!("open claim-bound module storage: {:?}", error.kind()))?;
            body(scoped)
        },
    )?;
    claimed
        .map(Some)
        .ok_or_else(|| anyhow::Error::new(ImportClaimHeld))
}

/// Another holder's live claim is importing this source. The import has not
/// happened, so it must be retried, never read as "nothing to import".
#[derive(Debug)]
pub(crate) struct ImportClaimHeld;

impl std::fmt::Display for ImportClaimHeld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another process holds this legacy import claim; retrying at the next call")
    }
}

impl std::error::Error for ImportClaimHeld {}

/// A stable marker id for one source path: `<prefix>@<16 hex of sha256(path)>`.
pub(super) fn marker_id(prefix: &str, source: &Path) -> String {
    let digest = sha2::Sha256::digest(source.as_os_str().as_encoded_bytes());
    format!("{prefix}@{}", &hex::encode(digest)[..16])
}

/// Whether the importer `kind` already finished for marker `id`, recorded in
/// the host-owned import-state collection of the module's scope. `kind` is a
/// stable marker family name (`tinybox_host_import`), outside the module's
/// granted collections.
pub(super) async fn already_imported(scoped: &Storage, kind: &str, id: &str) -> Result<bool> {
    crate::storage::import_state::host_import_done(scoped, kind, id)
        .await
        .map_err(|error| anyhow!("read the import state: {:?}", error.kind()))
}

/// Records that the import of `id` finished. Marking twice is harmless.
pub(super) async fn mark_imported(scoped: &Storage, kind: &str, id: &str) -> Result<()> {
    crate::storage::import_state::host_import_mark(scoped, kind, id)
        .await
        .map_err(|error| anyhow!("write the import state: {:?}", error.kind()))
}

/// Writes `doc` only when `id` is absent; `true` when it was written.
pub(super) async fn put_absent(
    docs: &Arc<dyn DocumentStore>,
    collection: &str,
    id: &str,
    doc: Value,
) -> Result<bool> {
    match docs.put(collection, id, doc, Precondition::Absent).await {
        Ok(_) => Ok(true),
        Err(error) if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) => {
            Ok(false)
        }
        Err(error) => Err(anyhow!("write {collection}: {:?}", error.kind())),
    }
}

/// Writes `doc` with owner metadata only when `id` is absent; `true` when it
/// was written.
pub(super) async fn put_absent_with(
    docs: &Arc<dyn DocumentStore>,
    collection: &str,
    id: &str,
    doc: Value,
    options: WriteOptions,
) -> Result<bool> {
    match docs
        .put_with(collection, id, doc, Precondition::Absent, options)
        .await
    {
        Ok(_) => Ok(true),
        Err(error) if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) => {
            Ok(false)
        }
        Err(error) => Err(anyhow!("write {collection}: {:?}", error.kind())),
    }
}

/// Wall clock in milliseconds since the Unix epoch.
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// Parses the non-empty lines of a JSONL file; a line that does not parse is
/// skipped (and counted) rather than failing the whole file.
pub(super) fn parse_jsonl(text: &str) -> (Vec<Value>, usize) {
    let mut values = Vec::new();
    let mut skipped = 0;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        match serde_json::from_str::<Value>(line) {
            Ok(value) if value.is_object() => values.push(value),
            _ => skipped += 1,
        }
    }
    (values, skipped)
}

/// Appends `values` to an empty `stream` in one batch (so a failed attempt
/// leaves it empty and a retry starts clean); a stream that already holds
/// records is left alone so a retry cannot duplicate them. Returns the number
/// appended.
pub(super) async fn append_if_empty(
    scoped: &Storage,
    stream: &str,
    values: Vec<Value>,
) -> Result<usize> {
    if values.is_empty() {
        return Ok(0);
    }
    let streams = scoped.streams();
    let held = streams
        .len(stream)
        .await
        .map_err(|error| anyhow!("read stream length: {:?}", error.kind()))?;
    if held != 0 {
        return Ok(0);
    }
    let total = values.len();
    streams
        .append_batch(stream, values)
        .await
        .map_err(|error| anyhow!("append to a stream: {:?}", error.kind()))?;
    Ok(total)
}
