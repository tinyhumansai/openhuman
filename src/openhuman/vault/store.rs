use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::openhuman::config::Config;

use super::types::{Vault, VaultFile, VaultFileStatus, VaultWriteState};

static MIGRATED_VAULT_DBS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

pub(crate) const VAULT_WRITE_REASON_EMPTY_PATH: &str = "empty_path";
pub(crate) const VAULT_WRITE_REASON_UNAVAILABLE: &str = "unavailable";
pub(crate) const VAULT_WRITE_REASON_NOT_DIRECTORY: &str = "not_directory";
pub(crate) const VAULT_WRITE_REASON_READ_ONLY: &str = "read_only";
pub(crate) const VAULT_WRITE_REASON_WRITABLE: &str = "writable";

pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = config.workspace_dir.join("vault").join("vault.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create vault directory: {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open vault DB: {}", db_path.display()))?;

    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS vaults (
            id              TEXT PRIMARY KEY,
            name            TEXT NOT NULL,
            root_path       TEXT NOT NULL,
            host_os         TEXT,
            namespace       TEXT NOT NULL UNIQUE,
            include_globs   TEXT NOT NULL DEFAULT '[]',
            exclude_globs   TEXT NOT NULL DEFAULT '[]',
            created_at      TEXT NOT NULL,
            last_synced_at  TEXT
         );
         CREATE TABLE IF NOT EXISTS vault_files (
            vault_id     TEXT NOT NULL,
            rel_path     TEXT NOT NULL,
            document_id  TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            mtime_ms     INTEGER NOT NULL,
            bytes        INTEGER NOT NULL,
            ingested_at  TEXT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'ok',
            PRIMARY KEY (vault_id, rel_path),
            FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_vault_files_vault ON vault_files(vault_id);",
    )
    .context("Failed to initialize vault schema")?;

    let migrated = MIGRATED_VAULT_DBS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut migrated_paths = migrated
        .lock()
        .map_err(|_| anyhow!("Failed to lock vault migration cache"))?;
    if !migrated_paths.contains(&db_path) {
        ensure_host_os_column(&conn).context("Failed to migrate vault schema")?;
        migrated_paths.insert(db_path.clone());
    }

    f(&conn)
}

pub fn insert_vault(config: &Config, vault: &Vault) -> Result<()> {
    insert_vault_inner(config, vault, true)
}

#[cfg(test)]
pub(crate) fn insert_vault_preserving_host_for_tests(config: &Config, vault: &Vault) -> Result<()> {
    insert_vault_inner(config, vault, false)
}

fn insert_vault_inner(config: &Config, vault: &Vault, stamp_current_host: bool) -> Result<()> {
    with_connection(config, |conn| {
        let host_os = normalized_host_os(vault.host_os.as_deref()).or_else(|| {
            if stamp_current_host {
                Some(current_host_os())
            } else {
                None
            }
        });
        conn.execute(
            "INSERT INTO vaults (id, name, root_path, host_os, namespace, include_globs, exclude_globs, created_at, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                vault.id,
                vault.name,
                vault.root_path,
                host_os,
                vault.namespace,
                serde_json::to_string(&vault.include_globs)?,
                serde_json::to_string(&vault.exclude_globs)?,
                vault.created_at.to_rfc3339(),
                vault.last_synced_at.map(|t| t.to_rfc3339()),
            ],
        )
        .context("Failed to insert vault")?;
        Ok(())
    })
}

pub fn list_vaults(config: &Config) -> Result<Vec<Vault>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT v.id, v.name, v.root_path, v.host_os, v.namespace, v.include_globs, v.exclude_globs,
                    v.created_at, v.last_synced_at,
                    (SELECT COUNT(*) FROM vault_files vf WHERE vf.vault_id = v.id AND vf.status = 'ok')
             FROM vaults v
             ORDER BY v.created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_vault)?;
        let mut out = Vec::new();
        for row in rows {
            let vault = row?;
            if vault_belongs_to_current_host(&vault) {
                out.push(vault);
            } else {
                log::debug!(
                    "[vault] hiding incompatible vault id={} host_os={:?}",
                    vault.id,
                    vault.host_os
                );
            }
        }
        Ok(out)
    })
}

pub fn get_vault(config: &Config, id: &str) -> Result<Option<Vault>> {
    with_connection(config, |conn| {
        let vault = conn
            .query_row(
                "SELECT v.id, v.name, v.root_path, v.host_os, v.namespace, v.include_globs, v.exclude_globs,
                    v.created_at, v.last_synced_at,
                    (SELECT COUNT(*) FROM vault_files vf WHERE vf.vault_id = v.id AND vf.status = 'ok')
             FROM vaults v WHERE v.id = ?1",
                params![id],
                row_to_vault,
            )
            .optional()
            .context("Failed to read vault")?;
        Ok(vault.filter(vault_belongs_to_current_host))
    })
}

pub fn remove_vault(config: &Config, id: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let n = conn
            .execute("DELETE FROM vaults WHERE id = ?1", params![id])
            .context("Failed to delete vault")?;
        Ok(n > 0)
    })
}

pub fn touch_last_synced(config: &Config, id: &str, when: DateTime<Utc>) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE vaults SET last_synced_at = ?2 WHERE id = ?1",
            params![id, when.to_rfc3339()],
        )?;
        Ok(())
    })
}

pub fn list_files(config: &Config, vault_id: &str) -> Result<Vec<VaultFile>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT vault_id, rel_path, document_id, content_hash, mtime_ms, bytes, ingested_at, status
             FROM vault_files WHERE vault_id = ?1 ORDER BY rel_path",
        )?;
        let rows = stmt.query_map(params![vault_id], row_to_file)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

pub fn upsert_file(config: &Config, file: &VaultFile) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO vault_files (vault_id, rel_path, document_id, content_hash, mtime_ms, bytes, ingested_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(vault_id, rel_path) DO UPDATE SET
                document_id = excluded.document_id,
                content_hash = excluded.content_hash,
                mtime_ms = excluded.mtime_ms,
                bytes = excluded.bytes,
                ingested_at = excluded.ingested_at,
                status = excluded.status",
            params![
                file.vault_id,
                file.rel_path,
                file.document_id,
                file.content_hash,
                file.mtime_ms,
                file.bytes as i64,
                file.ingested_at.to_rfc3339(),
                file.status.as_str(),
            ],
        )?;
        Ok(())
    })
}

pub fn delete_file(config: &Config, vault_id: &str, rel_path: &str) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "DELETE FROM vault_files WHERE vault_id = ?1 AND rel_path = ?2",
            params![vault_id, rel_path],
        )?;
        Ok(())
    })
}

fn row_to_vault(row: &rusqlite::Row<'_>) -> rusqlite::Result<Vault> {
    let include_raw: String = row.get(5)?;
    let exclude_raw: String = row.get(6)?;
    let created_raw: String = row.get(7)?;
    let last_raw: Option<String> = row.get(8)?;
    let file_count: i64 = row.get(9)?;
    let root_path: String = row.get(2)?;
    let (write_state, write_state_reason) = vault_write_state_for_root_path(&root_path);
    Ok(Vault {
        id: row.get(0)?,
        name: row.get(1)?,
        root_path,
        host_os: row.get(3)?,
        namespace: row.get(4)?,
        include_globs: serde_json::from_str(&include_raw).unwrap_or_default(),
        exclude_globs: serde_json::from_str(&exclude_raw).unwrap_or_default(),
        created_at: parse_dt(&created_raw),
        last_synced_at: last_raw.as_deref().map(parse_dt),
        file_count: file_count.max(0) as u64,
        write_state,
        write_state_reason,
    })
}

fn row_to_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<VaultFile> {
    let ingested_raw: String = row.get(6)?;
    let status_raw: String = row.get(7)?;
    let bytes: i64 = row.get(5)?;
    Ok(VaultFile {
        vault_id: row.get(0)?,
        rel_path: row.get(1)?,
        document_id: row.get(2)?,
        content_hash: row.get(3)?,
        mtime_ms: row.get(4)?,
        bytes: bytes.max(0) as u64,
        ingested_at: parse_dt(&ingested_raw),
        status: VaultFileStatus::parse(&status_raw),
    })
}

fn parse_dt(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn ensure_host_os_column(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(vaults)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut has_host_os = false;
    for column in columns {
        if column?.eq_ignore_ascii_case("host_os") {
            has_host_os = true;
            break;
        }
    }

    if !has_host_os {
        conn.execute("ALTER TABLE vaults ADD COLUMN host_os TEXT", [])?;
    }
    Ok(())
}

pub(crate) fn current_host_os() -> &'static str {
    std::env::consts::OS
}

pub(crate) fn vault_write_state_for_root_path(
    root_path: &str,
) -> (VaultWriteState, Option<String>) {
    let trimmed = root_path.trim();
    if trimmed.is_empty() {
        return (
            VaultWriteState::Unavailable,
            Some(VAULT_WRITE_REASON_EMPTY_PATH.to_string()),
        );
    }

    let path = std::path::Path::new(trimmed);
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return (
                VaultWriteState::Unavailable,
                Some(VAULT_WRITE_REASON_UNAVAILABLE.to_string()),
            )
        }
    };

    if !metadata.is_dir() {
        return (
            VaultWriteState::Unavailable,
            Some(VAULT_WRITE_REASON_NOT_DIRECTORY.to_string()),
        );
    }

    if metadata.permissions().readonly() {
        return (
            VaultWriteState::ReadOnly,
            Some(VAULT_WRITE_REASON_READ_ONLY.to_string()),
        );
    }

    (
        VaultWriteState::Writable,
        Some(VAULT_WRITE_REASON_WRITABLE.to_string()),
    )
}

pub(crate) fn vault_write_state_reason_message(reason_code: Option<&str>) -> &'static str {
    match reason_code {
        Some(VAULT_WRITE_REASON_EMPTY_PATH) => "Vault folder path is empty.",
        Some(VAULT_WRITE_REASON_UNAVAILABLE) => "Vault folder is not available on this device.",
        Some(VAULT_WRITE_REASON_NOT_DIRECTORY) => "Vault path is not a directory.",
        Some(VAULT_WRITE_REASON_READ_ONLY) => "Vault folder is read-only on this device.",
        Some(VAULT_WRITE_REASON_WRITABLE) => {
            "Approved markdown/wiki writes can be saved in this vault."
        }
        _ => "Vault write state is unknown.",
    }
}

pub(crate) fn path_looks_compatible_with_host_os(raw_path: &str, host_os: &str) -> bool {
    let path = raw_path.trim();
    if path.is_empty() {
        return false;
    }

    if is_windows_host_os(host_os) {
        return looks_like_windows_absolute_path(path);
    }

    looks_like_unix_absolute_path(path)
}

fn vault_belongs_to_current_host(vault: &Vault) -> bool {
    let current = current_host_os();
    let Some(host_os) = normalized_host_os(vault.host_os.as_deref()) else {
        return path_looks_compatible_with_host_os(&vault.root_path, current);
    };

    host_os.eq_ignore_ascii_case(current)
        && path_looks_compatible_with_host_os(&vault.root_path, current)
}

fn normalized_host_os(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|host_os| !host_os.is_empty())
}

fn is_windows_host_os(host_os: &str) -> bool {
    host_os.eq_ignore_ascii_case("windows") || host_os.eq_ignore_ascii_case("win32")
}

fn looks_like_windows_absolute_path(path: &str) -> bool {
    looks_like_windows_drive_path(path) || looks_like_windows_unc_path(path)
}

fn looks_like_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// Only backslash-style UNC (`\\server\share`). Forward-slash `//…` is
/// POSIX-legal and must not be classified as Windows.
fn looks_like_windows_unc_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0] == b'\\' && bytes[1] == b'\\' && !matches!(bytes[2], b'\\' | b'/')
}

fn looks_like_unix_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && !looks_like_windows_drive_path(path)
        && !looks_like_windows_unc_path(path)
}
