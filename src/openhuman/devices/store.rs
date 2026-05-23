//! SQLite persistence for paired devices.
//!
//! Follows the same `with_connection` pattern as `cron/store.rs`:
//! open a per-call connection to a domain-scoped `.db` file inside the
//! workspace directory, execute DDL on each open (idempotent), then run
//! the requested query and return.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};

use crate::openhuman::config::Config;
use crate::openhuman::devices::types::PairedDevice;

// ---------------------------------------------------------------------------
// Public store API
// ---------------------------------------------------------------------------

/// Persist a newly-paired device.
pub fn insert_device(
    config: &Config,
    channel_id: &str,
    label: &str,
    device_pubkey: &str,
    core_session_token_hash: &str,
) -> Result<PairedDevice> {
    let now = Utc::now().to_rfc3339();
    with_connection(config, |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO paired_devices \
             (channel_id, label, device_pubkey, core_session_token_hash, \
              shared_secret_encrypted, created_at, last_seen_at, revoked) \
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, 0)",
            params![
                channel_id,
                label,
                device_pubkey,
                core_session_token_hash,
                now
            ],
        )
        .context("insert_device: INSERT failed")?;
        Ok(())
    })?;
    get_device(config, channel_id)?.ok_or_else(|| anyhow::anyhow!("device not found after insert"))
}

/// Update `last_seen_at` for a device (called on `tunnel:peer-status` online events).
pub fn touch_device(config: &Config, channel_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE paired_devices SET last_seen_at = ?1 WHERE channel_id = ?2 AND revoked = 0",
            params![now, channel_id],
        )
        .context("touch_device: UPDATE failed")?;
        Ok(())
    })
}

/// Mark a device as revoked (soft delete).
pub fn revoke_device(config: &Config, channel_id: &str) -> Result<bool> {
    let rows = with_connection(config, |conn| {
        conn.execute(
            "UPDATE paired_devices SET revoked = 1 WHERE channel_id = ?1",
            params![channel_id],
        )
        .context("revoke_device: UPDATE failed")
    })?;
    Ok(rows > 0)
}

/// Load a single paired device by channel_id (returns None if not found).
pub fn get_device(config: &Config, channel_id: &str) -> Result<Option<PairedDevice>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT channel_id, label, device_pubkey, created_at, last_seen_at, revoked \
             FROM paired_devices WHERE channel_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![channel_id], map_device_row)?;
        rows.next().transpose().map_err(Into::into)
    })
}

/// List all non-revoked paired devices ordered by creation time.
pub fn list_devices(config: &Config) -> Result<Vec<PairedDevice>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT channel_id, label, device_pubkey, created_at, last_seen_at, revoked \
             FROM paired_devices WHERE revoked = 0 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], map_device_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn map_device_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PairedDevice> {
    Ok(PairedDevice {
        channel_id: row.get(0)?,
        label: row.get(1)?,
        device_pubkey: row.get(2)?,
        created_at: row.get(3)?,
        last_seen_at: row.get(4)?,
        peer_online: None, // populated from in-memory peer-status map, not SQLite
        revoked: row.get::<_, i64>(5)? != 0,
    })
}

fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("devices").join("devices.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create devices dir: {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("open devices DB: {}", db_path.display()))?;

    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS paired_devices (
             channel_id                  TEXT PRIMARY KEY,
             label                       TEXT NOT NULL,
             device_pubkey               TEXT NOT NULL,
             core_session_token_hash     TEXT NOT NULL,
             shared_secret_encrypted     BLOB,
             created_at                  TEXT NOT NULL,
             last_seen_at                TEXT,
             revoked                     INTEGER NOT NULL DEFAULT 0
         );",
    )
    .context("devices DDL failed")?;

    log::debug!(
        "[devices/store] connection opened path={}",
        db_path.display()
    );
    f(&conn)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = dir.into_path();
        config
    }

    #[test]
    fn insert_and_list_device() {
        let config = test_config();
        let device = insert_device(
            &config,
            "CHAN001",
            "iPhone 15",
            "pubkey_abc",
            "token_hash_xyz",
        )
        .unwrap();
        assert_eq!(device.channel_id, "CHAN001");
        assert_eq!(device.label, "iPhone 15");
        assert!(!device.revoked);

        let list = list_devices(&config).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].channel_id, "CHAN001");
    }

    #[test]
    fn revoke_device_marks_revoked() {
        let config = test_config();
        insert_device(&config, "CHAN002", "iPad", "pubkey_def", "hash_abc").unwrap();
        let ok = revoke_device(&config, "CHAN002").unwrap();
        assert!(ok);

        let list = list_devices(&config).unwrap();
        assert!(list.is_empty(), "revoked device should not appear in list");
    }

    #[test]
    fn touch_device_updates_last_seen_at() {
        let config = test_config();
        insert_device(&config, "CHAN003", "Watch", "pubkey_ghi", "hash_def").unwrap();
        touch_device(&config, "CHAN003").unwrap();
        let dev = get_device(&config, "CHAN003").unwrap().unwrap();
        assert!(dev.last_seen_at.is_some());
    }

    #[test]
    fn get_device_returns_none_for_missing() {
        let config = test_config();
        let result = get_device(&config, "MISSING").unwrap();
        assert!(result.is_none());
    }
}
