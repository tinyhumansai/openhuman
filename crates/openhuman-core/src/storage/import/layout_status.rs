//! Read-only compatibility reader for migration progress in old workspace DBs.
//!
//! The v2 driver no longer exposes the v1 schema registry. This reader stays
//! inside the legacy-import boundary and inspects only the historical
//! `tsd.migrations` document rows needed by `storage.status`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::Snapshot;
use crate::storage::rusqlite;

#[derive(Debug, Deserialize)]
struct LegacyProgress {
    owner: String,
    to: u32,
    done: bool,
}

pub(crate) type MigrationProgress = (Vec<(String, u32)>, Option<i64>);

/// Reads completed owner versions and the last completion timestamp from a
/// historical Tiny Storage Drivers database without opening it through the
/// current backend or modifying the source file.
pub(crate) fn read(path: &Path) -> Result<MigrationProgress> {
    let snapshot = Snapshot::of(path, "layout-status/source.db")?;
    let connection = rusqlite::Connection::open_with_flags(
        snapshot.db(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .with_context(|| format!("open snapshot {} read-only", snapshot.db().display()))?;
    let mut statement = connection
        .prepare(
            "SELECT body, updated_ms FROM docs \
             WHERE scope = ?1 AND coll = ?2 AND deleted = 0",
        )
        .context("prepare legacy migration progress query")?;
    let mut rows = statement
        .query(["local", "tsd.migrations"])
        .context("query legacy migration progress")?;
    let mut owners = BTreeMap::<String, u32>::new();
    let mut last = None::<i64>;
    while let Some(row) = rows.next().context("read legacy migration progress")? {
        let body: rusqlite::types::Value = row.get(0).context("read migration progress body")?;
        let updated_ms: i64 = row.get(1).context("read migration update timestamp")?;
        let Ok(body) = decode_body(body) else {
            continue;
        };
        let Ok(progress) = serde_json::from_str::<LegacyProgress>(&body) else {
            // Match the old status view: malformed or unrelated rows do not
            // prevent reporting valid migration progress.
            continue;
        };
        if progress.done && progress.to > 0 {
            let version = owners.entry(progress.owner).or_default();
            *version = (*version).max(progress.to);
            last = last.max((updated_ms > 0).then_some(updated_ms));
        }
    }
    Ok((owners.into_iter().collect(), last))
}

fn decode_body(body: rusqlite::types::Value) -> Result<String> {
    match body {
        rusqlite::types::Value::Text(text) => Ok(text),
        rusqlite::types::Value::Blob(bytes) if bytes.first() == Some(&1) => {
            let json = zstd::decode_all(&bytes[1..]).context("decompress legacy migration body")?;
            String::from_utf8(json).context("decode legacy migration body as UTF-8")
        }
        _ => anyhow::bail!("unsupported legacy migration body encoding"),
    }
}

#[cfg(test)]
#[path = "layout_status_tests.rs"]
mod tests;
