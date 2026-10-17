//! Steps 2 and 3: bring everything the v1 files hold into the v1 database,
//! then run the schema migrations over it.
//!
//! The v1 database is opened with its own URL (`sqlite:<workspace>/storage`),
//! not through the process's installed backend, so the migration works the
//! same from a stopped core (the CLI), a boot hook, and a test. Every importer
//! is the one the running core uses, handed that backend explicitly:
//!
//! - the four small stores, the tinyflows stores and the sessions
//!   ([`crate::storage::import::run_all`]);
//! - the memory domain's `memory/*.json` state values (`tsd.kv`) and the
//!   legacy conversation log;
//!
//! all into the `local` scope, the one the desktop operator's profile reads.
//! Every importer is idempotent and records its completion in
//! `tsd.migrations`, so a resumed migration repeats nothing. An importer that
//! fails fails the step: nothing may be archived-and-deleted that was not
//! imported.
//!
//! Left to the running core, on first use under layout v2, because their
//! source moves with the rest of the workspace into `files/` and the lazy
//! importer finds it there: the sub-agent lifecycle files and the computer site
//! memory (both under moved directories), and the module importers
//! (connectors, x402, tinybox, tinyjuice), which need a running module host.

use std::path::Path;
use std::sync::Arc;

use super::state::LayoutError;
use super::Target;
use crate::config::Config;
use crate::storage::{self, StorageBackend};

/// What the import step did, for the report.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ImportSummary {
    /// Domains whose importer ran (including ones with nothing to import).
    pub domains: usize,
    /// Memory state values imported.
    pub memory_state: Vec<String>,
}

/// The v1 database URL of the profile (`sqlite:<workspace>/storage`).
pub fn v1_url(target: &Target) -> String {
    format!(
        "sqlite:{}",
        target
            .workspace()
            .join(storage::config::DEFAULT_DIR)
            .display()
    )
}

/// `<workspace>/storage/storage.db`, the file the URL opens.
pub fn v1_db(target: &Target) -> std::path::PathBuf {
    target
        .workspace()
        .join(storage::config::DEFAULT_DIR)
        .join("storage.db")
}

async fn open_v1(target: &Target) -> Result<Arc<dyn StorageBackend>, LayoutError> {
    let url = v1_url(target);
    storage::open(&url)
        .await
        .map_err(|e| LayoutError::Other(anyhow::anyhow!("open the v1 database: {e}")))
}

/// Step 2. Runs every importer over the v1 database.
///
/// # Errors
///
/// An importer that failed, or the database could not be opened.
pub async fn import(target: &Target, config: &Config) -> Result<ImportSummary, LayoutError> {
    let workspace = target.workspace();
    if !workspace.is_dir() {
        return Ok(ImportSummary::default());
    }
    let backend = open_v1(target).await?;
    let config = Config {
        workspace_dir: workspace.clone(),
        ..config.clone()
    };
    let summary = tokio::task::spawn_blocking({
        let backend = Arc::clone(&backend);
        move || import_blocking(&backend, &config, &workspace)
    })
    .await
    .map_err(|e| LayoutError::Other(anyhow::anyhow!("the import task did not finish: {e}")))??;
    drop(backend);
    Ok(summary)
}

fn import_blocking(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    workspace: &Path,
) -> Result<ImportSummary, LayoutError> {
    let mut summary = ImportSummary::default();
    let mut failures = Vec::new();
    for report in storage::import::run_all(backend, config) {
        summary.domains += 1;
        if let Err(error) = &report.outcome {
            failures.push(format!("{}: {error:#}", report.domain));
        }
    }
    let scoped = backend
        .for_scope(&storage::tenant_scope(None))
        .map_err(|e| LayoutError::Other(anyhow::anyhow!("open the local scope: {e}")))?;
    match crate::memory::state_store::import_all(&scoped, workspace) {
        Ok(names) => summary.memory_state = names,
        Err(error) => failures.push(format!("memory state: {error:#}")),
    }
    if let Err(error) = crate::threads::store::backend::import_legacy_into(backend, workspace) {
        failures.push(format!("conversations: {error}"));
    }
    if failures.is_empty() {
        Ok(summary)
    } else {
        Err(LayoutError::Other(anyhow::anyhow!(
            "legacy import failed, nothing was archived or deleted: {}",
            failures.join("; ")
        )))
    }
}

/// Step 3. Runs the schema migrations over the imported database (what boot
/// does), then checkpoints its WAL so the `.db` file is whole.
///
/// # Errors
///
/// A migration that failed.
pub async fn migrate(target: &Target) -> Result<(), LayoutError> {
    let db = v1_db(target);
    if !target.workspace().is_dir() {
        return Ok(());
    }
    let backend = open_v1(target).await?;
    let report = storage::boot::prepare_and_migrate(&backend)
        .await
        .map_err(|e| LayoutError::Other(anyhow::anyhow!("schema migration: {e}")))?;
    drop(backend);
    if let Some((scope, step, message)) = report.failures.first() {
        return Err(LayoutError::Other(anyhow::anyhow!(
            "schema migration failed in scope {scope} step {step}: {message}"
        )));
    }
    if db.is_file() {
        super::db::checkpoint(&db)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "legacy_tests.rs"]
mod tests;
