//! Importing a v1 memory store into the selected engine.
//!
//! [`scan`] opens `<workspace>/memory/memory.db` read-only through
//! `tinymemory-import` and counts what it would import. [`start`] refuses to
//! run without explicit consent (importing uploads local data to the engine),
//! then runs in the background: a blocking reader walks the legacy store and
//! hands each item to the async side, which scrubs and stores it and advances
//! the resumable [`Checkpoint`]. Progress and the checkpoint persist in
//! `<workspace>/memory/import_state.json`, so a restarted import resumes where
//! the last one stopped (an item stored but not yet checkpointed is re-sent,
//! and the engine treats it as a replay).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use tinymemory::import::{Checkpoint, ImportedItem, LegacyWorkspace};
use tinymemory::ItemKind;

use crate::config::Config;

use super::engine::{self, BoundEngine};
use super::error::{MemoryError, MemoryResult};
use super::ops::store_on;
use super::types::{ImportCounts, ImportPhase, ImportScanView, ImportState};

/// Items stored between two checkpoint writes.
const CHECKPOINT_EVERY: u64 = 25;

/// Imports running now, per workspace.
static RUNNING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Default, Serialize, Deserialize)]
struct ImportFile {
    #[serde(default)]
    state: ImportState,
    #[serde(default)]
    checkpoint: Checkpoint,
}

fn file_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("memory").join("import_state.json")
}

fn read_file(workspace_dir: &Path) -> ImportFile {
    std::fs::read_to_string(file_path(workspace_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_file(workspace_dir: &Path, file: &ImportFile) {
    let path = file_path(workspace_dir);
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let json = serde_json::to_vec_pretty(file).map_err(std::io::Error::other)?;
            std::fs::write(&path, json)
        });
    if let Err(error) = result {
        tracing::warn!(error = %error, "[memory:import] writing import state failed");
    }
}

/// Counts what a legacy store at `workspace_dir` holds, or `None` when there
/// is no v1 store there. Blocking (SQLite).
pub fn count_legacy(workspace_dir: &Path) -> Option<ImportCounts> {
    let workspace = match LegacyWorkspace::open(workspace_dir) {
        Ok(workspace) => workspace,
        Err(error) => {
            tracing::debug!(error = %error, "[memory:import] no legacy store");
            return None;
        }
    };
    let mut counts = ImportCounts::default();
    for imported in workspace.items() {
        match imported {
            Ok(ImportedItem { item, .. }) => match item.kind() {
                ItemKind::Document => counts.documents += 1,
                ItemKind::Conversation => counts.conversations += 1,
                ItemKind::Learning => counts.learnings += 1,
            },
            Err(error) => {
                tracing::warn!(error = %error, "[memory:import] legacy store unreadable mid-scan");
                break;
            }
        }
    }
    Some(counts)
}

/// `memory_import_scan`.
pub async fn scan(config: &Config) -> MemoryResult<ImportScanView> {
    let workspace_dir = config.workspace_dir.clone();
    let counts = tokio::task::spawn_blocking(move || count_legacy(&workspace_dir))
        .await
        .map_err(|error| MemoryError::Engine(format!("import scan failed: {error}")))?;
    Ok(ImportScanView {
        found: counts.is_some(),
        counts,
    })
}

/// `memory_import_status`.
#[must_use]
pub fn status(config: &Config) -> ImportState {
    let mut state = read_file(&config.workspace_dir).state;
    let running = RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&config.workspace_dir);
    if state.phase == ImportPhase::Running && !running {
        state.phase = ImportPhase::Error;
        state.error = Some("the import was interrupted; start it again to resume".to_string());
    }
    state
}

/// `memory_import_start`: requires `consent`, memory on, and a legacy store.
pub async fn start(config: &Config, consent: bool) -> MemoryResult<ImportState> {
    if !consent {
        return Err(MemoryError::invalid(
            "importing uploads local memory to the selected engine; pass consent: true",
        ));
    }
    let bound = engine::resolve(config).engine()?;
    let workspace_dir = config.workspace_dir.clone();
    let claimed = RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(workspace_dir.clone());
    if !claimed {
        return Ok(status(config));
    }
    let scan_dir = workspace_dir.clone();
    let counts = tokio::task::spawn_blocking(move || count_legacy(&scan_dir))
        .await
        .ok()
        .flatten();
    let Some(counts) = counts else {
        RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&workspace_dir);
        return Err(MemoryError::invalid("no v1 memory store to import"));
    };
    let mut file = read_file(&workspace_dir);
    if file.state.phase == ImportPhase::Done {
        file = ImportFile::default();
    }
    file.state.phase = ImportPhase::Running;
    file.state.total = counts.documents + counts.conversations + counts.learnings;
    file.state.error = None;
    write_file(&workspace_dir, &file);
    let state = file.state.clone();
    tracing::info!(total = state.total, "[memory:import] import started");
    tokio::spawn(async move {
        run(&workspace_dir, &bound, file).await;
        RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&workspace_dir);
    });
    Ok(state)
}

async fn run(workspace_dir: &Path, bound: &BoundEngine, mut file: ImportFile) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<ImportedItem, String>>(16);
    let reader_dir = workspace_dir.to_path_buf();
    let checkpoint = file.checkpoint.clone();
    let reader = tokio::task::spawn_blocking(move || {
        let workspace = match LegacyWorkspace::open(&reader_dir) {
            Ok(workspace) => workspace,
            Err(error) => {
                let _ = tx.blocking_send(Err(error.to_string()));
                return;
            }
        };
        for imported in workspace.items_from(&checkpoint) {
            let message = imported.map_err(|error| error.to_string());
            let stop = message.is_err();
            if tx.blocking_send(message).is_err() || stop {
                return;
            }
        }
    });
    let mut since_checkpoint = 0u64;
    let mut failure = None;
    while let Some(next) = rx.recv().await {
        let imported = match next {
            Ok(imported) => imported,
            Err(error) => {
                failure = Some(format!("reading the legacy store failed: {error}"));
                break;
            }
        };
        match store_on(bound, imported.item).await {
            Ok(_) => {
                file.state.imported += 1;
                file.checkpoint = imported.checkpoint;
                since_checkpoint += 1;
                if since_checkpoint >= CHECKPOINT_EVERY {
                    write_file(workspace_dir, &file);
                    since_checkpoint = 0;
                }
            }
            Err(error @ (MemoryError::Unauthorized(_) | MemoryError::Off(_))) => {
                failure = Some(error.to_string());
                break;
            }
            Err(error) => {
                tracing::debug!(code = error.code(), "[memory:import] item skipped");
                file.checkpoint = imported.checkpoint;
            }
        }
    }
    drop(rx);
    let _ = reader.await;
    match failure {
        Some(error) => {
            tracing::warn!(
                imported = file.state.imported,
                "[memory:import] import stopped"
            );
            file.state.phase = ImportPhase::Error;
            file.state.error = Some(error);
        }
        None => {
            tracing::info!(
                imported = file.state.imported,
                "[memory:import] import finished"
            );
            file.state.phase = ImportPhase::Done;
            file.state.error = None;
        }
    }
    write_file(workspace_dir, &file);
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
