//! Read-only access to `~/Library/Messages/chat.db`.
//!
//! Opens the SQLite file with `SQLITE_OPEN_READ_ONLY` so we never mutate
//! user data and never take a write lock that could conflict with
//! Messages.app. The query is parameterised by a rowid cursor so each
//! tick pulls only new messages.

#![cfg(target_os = "macos")]

use std::path::Path;

use rusqlite::{params, Connection, OpenFlags};

/// One flattened message row joined across message/handle/chat tables.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Message {
    pub rowid: i64,
    pub guid: Option<String>,
    pub text: Option<String>,
    /// Binary NSKeyedArchiver/typedstream blob carrying message body for
    /// newer macOS versions that leave `text` NULL. Best-effort decoded at
    /// transcript-format time.
    pub attributed_body: Option<Vec<u8>>,
    /// Apple epoch nanoseconds (seconds since 2001-01-01 UTC × 1e9).
    pub date_ns: i64,
    pub is_from_me: bool,
    pub handle_id: Option<String>,
    pub chat_identifier: Option<String>,
    pub chat_name: Option<String>,
    pub service: Option<String>,
}

/// Open chat.db read-only. Returns a friendly error hint if Full Disk
/// Access is not granted (the typical failure mode on first run).
fn open(db_path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
    )
}

/// Read up to `limit` messages with `ROWID > since_rowid`, ordered by
/// ROWID ascending. Joins across message / handle / chat_message_join /
/// chat to produce one flat record per message.
pub fn read_since(db_path: &Path, since_rowid: i64, limit: usize) -> anyhow::Result<Vec<Message>> {
    let conn = open(db_path).map_err(|e| {
        anyhow::anyhow!(
            "open chat.db failed ({}). Grant Full Disk Access to OpenHuman: \
             System Settings → Privacy & Security → Full Disk Access.",
            e
        )
    })?;

    let mut stmt = conn.prepare(
        r#"
        SELECT
          m.ROWID            AS rowid,
          m.guid             AS guid,
          m.text             AS text,
          m.attributedBody   AS attributed_body,
          m.date             AS date_ns,
          m.is_from_me       AS is_from_me,
          h.id               AS handle_id,
          c.chat_identifier  AS chat_identifier,
          c.display_name     AS chat_name,
          m.service          AS service
        FROM message m
        LEFT JOIN handle h ON h.ROWID = m.handle_id
        LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
        LEFT JOIN chat c ON c.ROWID = cmj.chat_id
        WHERE m.ROWID > ?1
        ORDER BY m.ROWID ASC
        LIMIT ?2
        "#,
    )?;

    let rows = stmt.query_map(params![since_rowid, limit as i64], |row| {
        Ok(Message {
            rowid: row.get(0)?,
            guid: row.get(1)?,
            text: row.get(2)?,
            attributed_body: row.get(3)?,
            date_ns: row.get(4)?,
            is_from_me: row.get::<_, i64>(5)? != 0,
            handle_id: row.get(6)?,
            chat_identifier: row.get(7)?,
            chat_name: row.get(8)?,
            service: row.get(9)?,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Read ALL messages for a single `(chat_identifier, day)` slice, inclusive
/// of the day boundary in Apple nanosecond epoch. Used to rebuild full-day
/// transcripts before upserting memory docs — so tick-over-tick we always
/// write the complete conversation for the day, never a partial delta
/// that would overwrite prior content.
pub fn read_chat_day(
    db_path: &Path,
    chat_identifier: &str,
    day_start_apple_ns: i64,
    day_end_apple_ns: i64,
    limit: usize,
) -> anyhow::Result<Vec<Message>> {
    let conn = open(db_path)
        .map_err(|e| anyhow::anyhow!("open chat.db failed for full-day read ({})", e))?;

    let mut stmt = conn.prepare(
        r#"
        SELECT
          m.ROWID            AS rowid,
          m.guid             AS guid,
          m.text             AS text,
          m.attributedBody   AS attributed_body,
          m.date             AS date_ns,
          m.is_from_me       AS is_from_me,
          h.id               AS handle_id,
          c.chat_identifier  AS chat_identifier,
          c.display_name     AS chat_name,
          m.service          AS service
        FROM message m
        LEFT JOIN handle h ON h.ROWID = m.handle_id
        LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
        LEFT JOIN chat c ON c.ROWID = cmj.chat_id
        WHERE c.chat_identifier = ?1
          AND m.date >= ?2
          AND m.date <  ?3
        ORDER BY m.date ASC
        LIMIT ?4
        "#,
    )?;

    let rows = stmt.query_map(
        params![
            chat_identifier,
            day_start_apple_ns,
            day_end_apple_ns,
            limit as i64
        ],
        |row| {
            Ok(Message {
                rowid: row.get(0)?,
                guid: row.get(1)?,
                text: row.get(2)?,
                attributed_body: row.get(3)?,
                date_ns: row.get(4)?,
                is_from_me: row.get::<_, i64>(5)? != 0,
                handle_id: row.get(6)?,
                chat_identifier: row.get(7)?,
                chat_name: row.get(8)?,
                service: row.get(9)?,
            })
        },
    )?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}
