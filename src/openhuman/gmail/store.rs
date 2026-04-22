//! SQLite-backed store for Gmail account metadata and sync cursors.
//!
//! Uses the same workspace SQLite database pattern as other domains in
//! `src/openhuman/`. The table is created on first access (DDL is
//! idempotent — `CREATE TABLE IF NOT EXISTS`).

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};

use crate::openhuman::gmail::types::GmailAccount;

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// DDL that is executed once per connection open. Idempotent.
const CREATE_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS gmail_accounts (
    account_id      TEXT PRIMARY KEY NOT NULL,
    email           TEXT NOT NULL DEFAULT '',
    connected_at_ms INTEGER NOT NULL,
    last_sync_at_ms INTEGER NOT NULL DEFAULT 0,
    last_sync_count INTEGER NOT NULL DEFAULT 0,
    cron_job_id     TEXT
);
"#;

// ---------------------------------------------------------------------------
// Connection helper
// ---------------------------------------------------------------------------

fn open_db(config: &crate::openhuman::config::Config) -> Result<Connection> {
    let path = config.workspace_dir.join("gmail_accounts.db");
    log::debug!("[gmail][store] opening db at {}", path.display());
    let conn =
        Connection::open(&path).with_context(|| format!("open gmail db {}", path.display()))?;
    conn.execute_batch(CREATE_TABLE)
        .context("create gmail_accounts table")?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Public CRUD
// ---------------------------------------------------------------------------

/// Upsert an account record. Creates the row on first connect or refreshes
/// the email / cron_job_id on subsequent opens.
pub fn upsert_account(
    config: &crate::openhuman::config::Config,
    account: &GmailAccount,
) -> Result<()> {
    log::debug!(
        "[gmail][store] upsert account_id={} email={}",
        account.account_id,
        account.email
    );
    let conn = open_db(config)?;
    conn.execute(
        r#"INSERT INTO gmail_accounts
               (account_id, email, connected_at_ms, last_sync_at_ms, last_sync_count, cron_job_id)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6)
           ON CONFLICT(account_id) DO UPDATE SET
               email           = excluded.email,
               cron_job_id     = excluded.cron_job_id"#,
        params![
            account.account_id,
            account.email,
            account.connected_at_ms,
            account.last_sync_at_ms,
            account.last_sync_count,
            account.cron_job_id,
        ],
    )
    .context("upsert gmail account")?;
    Ok(())
}

/// Update the sync cursor for an account (last_sync_at_ms + last_sync_count).
pub fn update_sync_cursor(
    config: &crate::openhuman::config::Config,
    account_id: &str,
    count: i64,
) -> Result<()> {
    log::debug!(
        "[gmail][store] update_sync_cursor account_id={} count={}",
        account_id,
        count
    );
    let now_ms = Utc::now().timestamp_millis();
    let conn = open_db(config)?;
    conn.execute(
        "UPDATE gmail_accounts SET last_sync_at_ms = ?1, last_sync_count = ?2 WHERE account_id = ?3",
        params![now_ms, count, account_id],
    )
    .context("update sync cursor")?;
    Ok(())
}

/// Remove an account record entirely.
pub fn remove_account(config: &crate::openhuman::config::Config, account_id: &str) -> Result<()> {
    log::debug!("[gmail][store] remove account_id={}", account_id);
    let conn = open_db(config)?;
    conn.execute(
        "DELETE FROM gmail_accounts WHERE account_id = ?1",
        params![account_id],
    )
    .context("remove gmail account")?;
    Ok(())
}

/// Retrieve a single account by id.
pub fn get_account(
    config: &crate::openhuman::config::Config,
    account_id: &str,
) -> Result<Option<GmailAccount>> {
    let conn = open_db(config)?;
    let mut stmt = conn.prepare(
        "SELECT account_id, email, connected_at_ms, last_sync_at_ms, last_sync_count, cron_job_id
           FROM gmail_accounts WHERE account_id = ?1",
    )?;
    let mut rows = stmt.query(params![account_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_account(&row)?))
    } else {
        Ok(None)
    }
}

/// List all connected accounts, ordered by connected_at_ms desc.
pub fn list_accounts(config: &crate::openhuman::config::Config) -> Result<Vec<GmailAccount>> {
    let conn = open_db(config)?;
    let mut stmt = conn.prepare(
        "SELECT account_id, email, connected_at_ms, last_sync_at_ms, last_sync_count, cron_job_id
           FROM gmail_accounts ORDER BY connected_at_ms DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        row_to_account(row).map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("list gmail accounts")
}

// ---------------------------------------------------------------------------
// Row mapper
// ---------------------------------------------------------------------------

fn row_to_account(row: &rusqlite::Row<'_>) -> Result<GmailAccount> {
    Ok(GmailAccount {
        account_id: row.get(0)?,
        email: row.get(1)?,
        connected_at_ms: row.get(2)?,
        last_sync_at_ms: row.get(3)?,
        last_sync_count: row.get(4)?,
        cron_job_id: row.get(5)?,
    })
}
