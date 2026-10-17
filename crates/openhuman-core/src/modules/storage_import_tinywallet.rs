//! One-time, read-only import of the x402 payments ledger into the wallet
//! module's storage.
//!
//! Before the storage contract the module's ledger kept every payment attempt
//! as one JSON line in `<workspace>/x402/payments.jsonl`; a payment that
//! changed status was appended again with the same id. From v0.22.0 the same
//! records live in the `x402.payments` stream of the module's own scope, and
//! the module no longer reads the file. Dropping it would forget payments that
//! count against the daily and monthly budget, so an upgraded install would
//! silently get its spending allowance back.
//!
//! * **The legacy file is never modified.** It is only read.
//! * **Nothing is duplicated.** The records are appended in one batch and only
//!   to an empty stream, so a retry after a failed attempt cannot repeat them
//!   and records the module wrote since are never preceded by old ones.
//! * **Runs once per workspace.** A marker document (in a collection the
//!   module's grant does not include) records it. An unreadable file writes no
//!   marker, so the next start retries; a line that does not parse is skipped.
//! * **What is not carried over.** Budget holds. The legacy ledger held
//!   reservations only in memory, so there is nothing on disk to import; the
//!   `x402_budget` document starts empty.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::json;
use tinystoragedrivers::ScopedStorage;

use super::storage_grant_schemas::tinywallet::{MODULE_ID, PAYMENTS_STREAM};
use super::storage_import_support as support;
use crate::core::runtime::DispatchCtx;

const LOG_PREFIX: &str = "[x402-import]";
const LEDGER_FILE: &str = "x402/payments.jsonl";
/// Host-only: not in the module's grant, so the module cannot see it.
const MARKER_COLLECTION: &str = "tinywallet_host_import";

/// How one import attempt ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// No storage backend is installed; the module is not using this storage.
    NoBackend,
    /// The marker says this workspace was already imported.
    AlreadyDone,
    /// There is no legacy ledger file.
    NoSource,
    /// The file was read and its records written.
    Imported { payments: usize, skipped: usize },
}

/// Imports the legacy ledger under `workspace_dir` into the module's scope of
/// the installed backend.
pub(crate) async fn import_legacy(ctx: &DispatchCtx, workspace_dir: &Path) -> Result<Outcome> {
    let Some(scoped) = support::module_scope(ctx, MODULE_ID)? else {
        return Ok(Outcome::NoBackend);
    };
    import_into(ctx, &scoped, workspace_dir).await
}

async fn import_into(
    ctx: &DispatchCtx,
    scoped: &ScopedStorage,
    workspace_dir: &Path,
) -> Result<Outcome> {
    let docs = scoped.documents();
    let marker = support::marker_id("x402", workspace_dir);
    if support::already_imported(scoped, MARKER_COLLECTION, &marker).await? {
        return Ok(Outcome::AlreadyDone);
    }
    let ledger = workspace_dir.join(LEDGER_FILE);
    if !ledger.is_file() {
        return Ok(Outcome::NoSource);
    }
    let (records, skipped) = ctx
        .spawn_blocking(move || -> Result<_> {
            let text = std::fs::read_to_string(&ledger)
                .with_context(|| format!("read {}", ledger.display()))?;
            Ok(support::parse_jsonl(&text))
        })
        .await
        .context("the legacy reader panicked")??;

    let payments = support::append_if_empty(scoped, PAYMENTS_STREAM, records)
        .await
        .map_err(|error| anyhow!("import the payments: {error:#}"))?;
    support::mark_imported(scoped, MARKER_COLLECTION, &marker).await?;
    tracing::info!("{LOG_PREFIX} imported payments={payments} skipped={skipped}");
    Ok(Outcome::Imported { payments, skipped })
}

#[cfg(test)]
#[path = "storage_import_tinywallet_tests.rs"]
mod tests;
