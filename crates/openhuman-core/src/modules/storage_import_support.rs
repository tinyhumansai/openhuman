//! What the one-time legacy importers (`storage_import_*`) share.
//!
//! Each importer moves a module's pre-storage files into the module's own
//! scope of the installed backend, once per source directory, never touching
//! the files. The shared parts are opening that scope, the host-only marker
//! that records a finished import, and a write that never replaces a record
//! the module has since written.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::Value;
use sha2::Digest;
use tinystoragedrivers::{DocumentStore, ErrorKind, Precondition, ScopedStorage};

/// The module's scope of the installed backend, or `None` when the process
/// runs without a storage backend (the module is then not using this storage).
pub(super) fn module_scope(
    ctx: &crate::core::runtime::DispatchCtx,
    module_id: &str,
) -> Result<Option<ScopedStorage>> {
    let Some(backend) = crate::storage::installed() else {
        return Ok(None);
    };
    let scope = super::storage_host::current_module_scope(ctx, module_id)
        .map_err(|error| anyhow!("resolve the module scope: {:?}", error.kind()))?;
    let scoped = backend
        .for_scope(&scope)
        .map_err(|error| anyhow!("open the module scope: {:?}", error.kind()))?;
    Ok(Some(scoped))
}

/// A stable marker id for one source path: `<prefix>@<16 hex of sha256(path)>`.
pub(super) fn marker_id(prefix: &str, source: &Path) -> String {
    let digest = sha2::Sha256::digest(source.as_os_str().as_encoded_bytes());
    format!("{prefix}@{}", &hex::encode(digest)[..16])
}

/// Whether the importer `kind` already finished for marker `id`, recorded in
/// `tsd.migrations` of the module's scope. `kind` is the old marker
/// collection name (`tinybox_host_import`), so a marker row an older build
/// wrote ([`crate::storage::import_state::migrate_legacy_markers`]) answers
/// the same question.
pub(super) async fn already_imported(scoped: &ScopedStorage, kind: &str, id: &str) -> Result<bool> {
    crate::storage::import_state::host_import_done(scoped, kind, id)
        .await
        .map_err(|error| anyhow!("read the import state: {:?}", error.kind()))
}

/// Records that the import of `id` finished. Marking twice is harmless.
pub(super) async fn mark_imported(scoped: &ScopedStorage, kind: &str, id: &str) -> Result<()> {
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
    scoped: &ScopedStorage,
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
