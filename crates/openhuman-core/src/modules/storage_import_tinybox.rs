//! One-time, read-only import of a legacy jail registry index into the
//! tinybox module's storage.
//!
//! A `JailRegistry` used to keep its metadata for every jail in
//! `<base>/index.json` (the documented base is `<workspace>/jails/`). From
//! v0.1.29 the index is one document per registry in the
//! `tinybox_jail_index` collection of the module's own scope, with the same
//! JSON shape, so a registry opened on the host's storage sees the jails an
//! older install created.
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

use anyhow::{Context, Result};
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, ScopedStorage};

use super::storage_grant_schemas::tinybox::{JAIL_INDEX_COLLECTION, MODULE_ID};
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
    let Some(scoped) = support::module_scope(ctx, MODULE_ID)? else {
        return Ok(Outcome::NoBackend);
    };
    import_into(&scoped, &workspace_dir.join(BASE_DIR)).await
}

async fn import_into(scoped: &ScopedStorage, base: &Path) -> Result<Outcome> {
    let docs = scoped.documents();
    let marker = support::marker_id("tinybox", base);
    if support::already_imported(scoped, MARKER_COLLECTION, &marker).await? {
        return Ok(Outcome::AlreadyDone);
    }
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
            docs.ensure_collection(&CollectionSpec::new(JAIL_INDEX_COLLECTION))
                .await
                .map_err(|error| {
                    anyhow::anyhow!("declare {JAIL_INDEX_COLLECTION}: {:?}", error.kind())
                })?;
            written = support::put_absent(docs, JAIL_INDEX_COLLECTION, &registry, doc).await?;
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
