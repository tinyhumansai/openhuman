//! One-time, read-only import of a legacy jail registry index into the
//! tinybox module's storage.
//!
//! A `JailRegistry` used to keep its metadata for every jail in
//! `<base>/index.json` (the documented base is `<workspace>/jails/`). The
//! importer converts that registry JSON through the TinyBox bus contract and
//! writes the current `box.jail_index` body, so a registry opened on the
//! host's storage sees jails an older install created.
//!
//! * **The legacy file is never modified.** It is only read.
//! * **Nothing is overwritten.** The document is written only when absent, so
//!   an index the module has written since is never replaced.
//! * **Runs once per base directory.** A marker document (in a collection the
//!   module's grant does not include) records it. An unreadable file writes no
//!   marker, so the next start retries; a file that is not a JSON object is
//!   skipped, as the registry itself would refuse it.
//! * **What is not carried over.** Box records: the module held them in memory
//!   before this release, so there is no file to read. The `tinybox` command
//!   line keeps its own `boxes.json` and is not the host's to migrate.

use std::path::Path;

use crate::storage::{CollectionSpec, Storage};
use anyhow::{Context, Result};
use serde_json::Value;
use tinystoragedrivers_types::WriteOptions;

use super::storage_grant_schemas::tinybox::MODULE_ID;
use super::storage_import_support as support;
use crate::core::runtime::DispatchCtx;

const LOG_PREFIX: &str = "[tinybox-import]";
/// The registry the module opens over the workspace's jails directory.
const BASE_DIR: &str = "jails";
const INDEX_FILE: &str = "index.json";
/// Host-only: not in the module's grant, so the module cannot see it.
const MARKER_COLLECTION: &str = "tinybox_host_import";

/// How one import attempt ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// No storage backend is installed; the module is not using this storage.
    NoBackend,
    /// The marker says this base directory was already imported.
    AlreadyDone,
    /// There is no legacy index file.
    NoSource,
    /// The index was read; `written` is false when a document already existed
    /// or the file was not a JSON object.
    Imported { written: bool },
}

/// Imports the index under `<workspace_dir>/jails` into the module's scope of
/// the installed backend.
pub(crate) async fn import_legacy(ctx: &DispatchCtx, workspace_dir: &Path) -> Result<Outcome> {
    if support::module_scope(ctx, MODULE_ID)?.is_none() {
        return Ok(Outcome::NoBackend);
    }
    let base = workspace_dir.join(BASE_DIR);
    let index = base.join(INDEX_FILE);
    if !index.is_file() {
        return Ok(Outcome::NoSource);
    }
    let marker = support::marker_id("tinybox", &base);
    let result =
        support::with_module_claim(ctx, MODULE_ID, MARKER_COLLECTION, &marker, move |scoped| {
            crate::storage::block_on_anyhow(async move { import_into(&scoped, &base).await })
        })?;
    let Some(outcome) = result else {
        return Ok(Outcome::NoBackend);
    };
    Ok(outcome)
}

async fn import_into(scoped: &Storage, base: &Path) -> Result<Outcome> {
    let marker = support::marker_id("tinybox", base);
    if support::already_imported(scoped, MARKER_COLLECTION, &marker).await? {
        return Ok(Outcome::AlreadyDone);
    }
    let docs = scoped.documents();
    let index = base.join(INDEX_FILE);
    if !index.is_file() {
        return Ok(Outcome::NoSource);
    }
    let text = tokio::fs::read_to_string(&index)
        .await
        .with_context(|| format!("read {}", index.display()))?;
    let registry = base
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(BASE_DIR)
        .to_owned();

    let mut written = false;
    match serde_json::from_str::<Value>(&text) {
        Ok(doc) if doc.is_object() => {
            let collection = tinybox_bus::storage::JAIL_INDEX_COLLECTION;
            docs.ensure_collection(&CollectionSpec::new(collection))
                .await
                .map_err(|error| anyhow::anyhow!("declare {collection}: {:?}", error.kind()))?;
            let body = tinybox_bus::storage::jail_index::to_body(doc)
                .context("convert the jail index into the current owner body")?;
            written = support::put_absent_with(
                docs,
                collection,
                &registry,
                body,
                WriteOptions::new().schema(tinybox_bus::storage::BODY_VERSION),
            )
            .await?;
        }
        _ => tracing::warn!("{LOG_PREFIX} skipped an index file that is not a JSON object"),
    }

    support::mark_imported(scoped, MARKER_COLLECTION, &marker).await?;
    tracing::info!("{LOG_PREFIX} imported written={written}");
    Ok(Outcome::Imported { written })
}

#[cfg(test)]
#[path = "storage_import_tinybox_tests.rs"]
mod tests;
