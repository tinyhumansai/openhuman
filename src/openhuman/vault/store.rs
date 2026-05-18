use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::openhuman::config::Config;

use super::types::{Vault, VaultFile, VaultFileStatus};

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

    f(&conn)
}

pub fn insert_vault(config: &Config, vault: &Vault) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO vaults (id, name, root_path, namespace, include_globs, exclude_globs, created_at, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                vault.id,
                vault.name,
                vault.root_path,
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
            "SELECT v.id, v.name, v.root_path, v.namespace, v.include_globs, v.exclude_globs,
                    v.created_at, v.last_synced_at,
                    (SELECT COUNT(*) FROM vault_files vf WHERE vf.vault_id = v.id AND vf.status = 'ok')
             FROM vaults v
             ORDER BY v.created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_vault)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

pub fn get_vault(config: &Config, id: &str) -> Result<Option<Vault>> {
    with_connection(config, |conn| {
        conn.query_row(
            "SELECT v.id, v.name, v.root_path, v.namespace, v.include_globs, v.exclude_globs,
                    v.created_at, v.last_synced_at,
                    (SELECT COUNT(*) FROM vault_files vf WHERE vf.vault_id = v.id AND vf.status = 'ok')
             FROM vaults v WHERE v.id = ?1",
            params![id],
            row_to_vault,
        )
        .optional()
        .context("Failed to read vault")
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
    let include_raw: String = row.get(4)?;
    let exclude_raw: String = row.get(5)?;
    let created_raw: String = row.get(6)?;
    let last_raw: Option<String> = row.get(7)?;
    let file_count: i64 = row.get(8)?;
    Ok(Vault {
        id: row.get(0)?,
        name: row.get(1)?,
        root_path: row.get(2)?,
        namespace: row.get(3)?,
        include_globs: serde_json::from_str(&include_raw).unwrap_or_default(),
        exclude_globs: serde_json::from_str(&exclude_raw).unwrap_or_default(),
        created_at: parse_dt(&created_raw),
        last_synced_at: last_raw.as_deref().map(parse_dt),
        file_count: file_count.max(0) as u64,
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
