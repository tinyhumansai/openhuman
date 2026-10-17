//! One-time, read-only import of tinyconnectors' legacy state files into the
//! module's storage.
//!
//! Before the storage contract the module kept two things under
//! `<workspace>/state/`: the user's scope preferences as one JSON file per
//! toolkit (`composio-user-scopes/<toolkit>.json`, the bare preference value)
//! and the webhook trigger archive as one JSONL file per UTC day
//! (`triggers/<YYYY-MM-DD>.jsonl`). From v0.18.0 it keeps them in its own scope
//! of the host's storage, as `composio-user-scopes` documents and
//! `triggers/<day>` streams, and no longer reads the files, so an install
//! upgraded with preferences set (a denied write, say) must not lose them.
//!
//! * **Legacy files are never modified.** They are only read.
//! * **Nothing is overwritten.** A preference document that already exists is
//!   kept, and a day's stream that already holds records is not appended to,
//!   so a retry cannot duplicate and records the module wrote since are never
//!   replaced.
//! * **Runs once per state directory.** A marker document (in a collection the
//!   module's grant does not include) records it. An unreadable file writes no
//!   marker, so the next start retries. A file whose content is not valid JSON
//!   is skipped, as the module itself would have refused it.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tinystoragedrivers::{CollectionSpec, ScopedStorage};

use super::storage_grant_schemas::tinyconnectors::{
    MODULE_ID, PREFS_COLLECTION, TRIGGER_STREAM_PREFIX,
};
use super::storage_import_support as support;
use crate::core::runtime::DispatchCtx;

const LOG_PREFIX: &str = "[connectors-import]";
const PREFS_DIR: &str = "composio-user-scopes";
const TRIGGERS_DIR: &str = "triggers";
/// Host-only: not in the module's grant, so the module cannot see it.
const MARKER_COLLECTION: &str = "tinyconnectors_host_import";

/// How one import attempt ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// No storage backend is installed; the module is not using this storage.
    NoBackend,
    /// The marker says this state directory was already imported.
    AlreadyDone,
    /// Neither legacy directory exists.
    NoSource,
    /// The files were read and their records written.
    Imported {
        prefs: usize,
        triggers: usize,
        skipped: usize,
    },
}

/// Imports the legacy files under `state_dir` into the module's scope of the
/// installed backend.
pub(crate) async fn import_legacy(ctx: &DispatchCtx, state_dir: &Path) -> Result<Outcome> {
    let Some(scoped) = support::module_scope(ctx, MODULE_ID)? else {
        return Ok(Outcome::NoBackend);
    };
    import_into(ctx, &scoped, state_dir).await
}

/// A preference file: the toolkit key and its bare JSON value.
struct LegacyPref {
    key: String,
    value: Value,
}

/// One day's archive: the stream it goes to and its parsed records.
struct LegacyDay {
    stream: String,
    records: Vec<Value>,
}

struct Legacy {
    prefs: Vec<LegacyPref>,
    days: Vec<LegacyDay>,
    skipped: usize,
}

async fn import_into(
    ctx: &DispatchCtx,
    scoped: &ScopedStorage,
    state_dir: &Path,
) -> Result<Outcome> {
    let docs = scoped.documents();
    let marker = support::marker_id("connectors", state_dir);
    if support::already_imported(scoped, MARKER_COLLECTION, &marker).await? {
        return Ok(Outcome::AlreadyDone);
    }
    let prefs_dir = state_dir.join(PREFS_DIR);
    let triggers_dir = state_dir.join(TRIGGERS_DIR);
    if !prefs_dir.is_dir() && !triggers_dir.is_dir() {
        return Ok(Outcome::NoSource);
    }

    let legacy = ctx
        .spawn_blocking(move || read_legacy(&prefs_dir, &triggers_dir))
        .await
        .context("the legacy reader panicked")??;

    docs.ensure_collection(&CollectionSpec::new(PREFS_COLLECTION))
        .await
        .map_err(|error| anyhow!("declare {PREFS_COLLECTION}: {:?}", error.kind()))?;
    let mut prefs = 0usize;
    for pref in legacy.prefs {
        let doc = json!({ "value": pref.value });
        if support::put_absent(docs, PREFS_COLLECTION, &pref.key, doc).await? {
            prefs += 1;
        }
    }
    let mut triggers = 0usize;
    for day in legacy.days {
        triggers += support::append_if_empty(scoped, &day.stream, day.records).await?;
    }

    support::mark_imported(scoped, MARKER_COLLECTION, &marker).await?;
    tracing::info!(
        "{LOG_PREFIX} imported prefs={prefs} triggers={triggers} skipped={}",
        legacy.skipped
    );
    Ok(Outcome::Imported {
        prefs,
        triggers,
        skipped: legacy.skipped,
    })
}

/// The files of `dir` with `extension`, sorted by name; a missing directory has
/// none.
fn files_with_extension(dir: &Path, extension: &str) -> Result<Vec<std::path::PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("list {}", dir.display())),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("list {}", dir.display()))?
            .path();
        if path.is_file() && path.extension().is_some_and(|found| found == extension) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn read_legacy(prefs_dir: &Path, triggers_dir: &Path) -> Result<Legacy> {
    let mut skipped = 0usize;
    let mut prefs = Vec::new();
    for path in files_with_extension(prefs_dir, "json")? {
        let Some(key) = path.file_stem().and_then(|stem| stem.to_str()) else {
            skipped += 1;
            continue;
        };
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => prefs.push(LegacyPref {
                key: key.to_owned(),
                value,
            }),
            Err(_) => {
                tracing::warn!("{LOG_PREFIX} skipped a preference file that is not valid JSON");
                skipped += 1;
            }
        }
    }
    let mut days = Vec::new();
    for path in files_with_extension(triggers_dir, "jsonl")? {
        let Some(day) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| is_day(stem))
        else {
            skipped += 1;
            continue;
        };
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let (records, bad) = support::parse_jsonl(&text);
        skipped += bad;
        days.push(LegacyDay {
            stream: format!("{TRIGGER_STREAM_PREFIX}{day}"),
            records,
        });
    }
    Ok(Legacy {
        prefs,
        days,
        skipped,
    })
}

/// `YYYY-MM-DD`, the only name the archive ever wrote.
fn is_day(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

#[cfg(test)]
#[path = "storage_import_tinyconnectors_tests.rs"]
mod tests;
