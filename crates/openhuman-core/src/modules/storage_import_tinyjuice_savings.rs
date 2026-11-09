//! One-time import of tinyjuice's legacy savings snapshot.
//!
//! The savings totals used to live in `<workspace>/state/tokenjuice_savings.json`
//! (a camelCase `SavingsAggregate`). The module now keeps them as one document
//! in its own `juice.savings` collection, so the host reads the old file and
//! hands the aggregate to the module (`ImportSavings`), which adopts it only
//! while it has recorded nothing.
//!
//! * **The file is only read.** It is never modified or deleted here; the
//!   layout migration archives it with the rest of `state/` (classified `Move`,
//!   "runtime state files", in `storage/layout/plan.rs`).
//! * **The path is fixed.** It is `state/tokenjuice_savings.json` under the
//!   workspace, read without following a symlink and capped in size; nothing
//!   in the file can name another path.
//! * **Runs once per file.** A host-owned marker in the module scope records
//!   it. A
//!   snapshot that does not parse is marked too (the old ledger ignored it as
//!   well), so a corrupt file is not retried forever. A failed hand-over writes
//!   no marker and is retried at the next install.

use std::future::Future;
use std::path::{Path, PathBuf};

use crate::storage::Storage;
use anyhow::{Context, Result};
use tinyjuice_bus::savings::SavingsAggregate;

use super::storage_grant_schemas::tinyjuice::MODULE_ID;
use super::storage_import_support as support;
use super::storage_import_tinyjuice::read_regular;
use crate::core::runtime::DispatchCtx;

const LOG_PREFIX: &str = "[tinyjuice-savings-import]";
/// Marker collection name, shared with the CCR importer's host-only family.
const MARKER_KIND: &str = "tinyjuice_host_import";
/// A real snapshot is a few KiB; anything past this is not one.
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// The legacy snapshot's location under `workspace`.
pub(crate) fn legacy_file(workspace: &Path) -> PathBuf {
    workspace.join("state").join("tokenjuice_savings.json")
}

/// How one import attempt ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// No storage backend is installed; the module is not using this storage.
    NoBackend,
    /// The marker says this file was already imported.
    AlreadyDone,
    /// There is no snapshot file.
    NoSource,
    /// The file was handled; `adopted` is whether the module took its totals.
    Imported { adopted: bool },
}

/// Hands the snapshot at `file` to the module through `adopt`.
pub(crate) async fn import_legacy<F, Fut>(
    ctx: &DispatchCtx,
    file: &Path,
    adopt: F,
) -> Result<Outcome>
where
    F: FnOnce(SavingsAggregate) -> Fut + Send + 'static,
    Fut: Future<Output = Result<bool>> + Send + 'static,
{
    if support::module_scope(ctx, MODULE_ID)?.is_none() {
        return Ok(Outcome::NoBackend);
    }
    if std::fs::symlink_metadata(file).is_err() {
        return Ok(Outcome::NoSource);
    }
    let marker = support::marker_id("savings", file);
    let file = file.to_owned();
    let result = support::with_module_claim(ctx, MODULE_ID, MARKER_KIND, &marker, move |scoped| {
        crate::storage::block_on_anyhow(async move { import_into(&scoped, &file, adopt).await })
    })?;
    let Some(outcome) = result else {
        return Ok(Outcome::NoBackend);
    };
    Ok(outcome)
}

async fn import_into<F, Fut>(scoped: &Storage, file: &Path, adopt: F) -> Result<Outcome>
where
    F: FnOnce(SavingsAggregate) -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    let marker = support::marker_id("savings", file);
    if support::already_imported(scoped, MARKER_KIND, &marker).await? {
        return Ok(Outcome::AlreadyDone);
    }
    if std::fs::symlink_metadata(file).is_err() {
        return Ok(Outcome::NoSource);
    }
    let path = file.to_owned();
    let bytes = crate::core::runtime::DispatchCtx::ambient()
        .spawn_blocking(move || read_regular(&path))
        .await
        .context("the legacy reader panicked")??;
    let Some((_, bytes)) = bytes else {
        return Ok(Outcome::NoSource);
    };
    let parsed = if bytes.len() > MAX_BYTES {
        None
    } else {
        serde_json::from_slice::<SavingsAggregate>(&bytes).ok()
    };
    let adopted = match parsed {
        Some(aggregate) => adopt(aggregate).await?,
        None => {
            log::warn!("{LOG_PREFIX} snapshot unreadable or too large, skipped");
            false
        }
    };
    support::mark_imported(scoped, MARKER_KIND, &marker).await?;
    log::debug!("{LOG_PREFIX} adopted={adopted}");
    Ok(Outcome::Imported { adopted })
}

#[cfg(test)]
#[path = "storage_import_tinyjuice_savings_tests.rs"]
mod tests;
