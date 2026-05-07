//! SQLite-backed persistence for structured WhatsApp Web data.
//!
//! Data is stored in a dedicated `whatsapp_data.db` file inside the
//! workspace directory. Tables: `wa_chats` and `wa_messages`.
//!
//! This store is local-only; no data is transmitted to external services.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::openhuman::whatsapp_data::types::{
    ChatMeta, IngestMessage, ListChatsRequest, ListMessagesRequest, SearchMessagesRequest,
    WhatsAppChat, WhatsAppMessage,
};

/// SQLite-backed store for WhatsApp chats and messages.
pub struct WhatsAppDataStore {
    db_path: std::path::PathBuf,
}

impl WhatsAppDataStore {
    /// Open or create the `whatsapp_data.db` SQLite database in `workspace_dir`.
    /// The directory (and any parents) are created if they do not exist.
    pub fn new(workspace_dir: &Path) -> Result<Self> {
        let db_path = workspace_dir.join("whatsapp_data").join("whatsapp_data.db");
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create whatsapp_data dir: {}", parent.display()))?;
        }
        log::debug!("[whatsapp_data] opening store at {}", db_path.display());
        let store = Self { db_path };
        store.init_schema()?;
        Ok(store)
    }

    /// Initialize the schema. Idempotent — safe to call on every startup.
    fn init_schema(&self) -> Result<()> {
        let conn = self.open_conn()?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS wa_chats (
                 account_id      TEXT NOT NULL,
                 chat_id         TEXT NOT NULL,
                 display_name    TEXT NOT NULL DEFAULT '',
                 is_group        INTEGER NOT NULL DEFAULT 0,
                 last_message_ts INTEGER NOT NULL DEFAULT 0,
                 message_count   INTEGER NOT NULL DEFAULT 0,
                 updated_at      INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (account_id, chat_id)
             );

             CREATE TABLE IF NOT EXISTS wa_messages (
                 account_id   TEXT NOT NULL,
                 chat_id      TEXT NOT NULL,
                 message_id   TEXT NOT NULL,
                 sender       TEXT NOT NULL DEFAULT '',
                 sender_jid   TEXT,
                 from_me      INTEGER NOT NULL DEFAULT 0,
                 body         TEXT NOT NULL DEFAULT '',
                 timestamp    INTEGER NOT NULL DEFAULT 0,
                 message_type TEXT,
                 source       TEXT NOT NULL DEFAULT '',
                 ingested_at  INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (account_id, chat_id, message_id)
             );
             CREATE INDEX IF NOT EXISTS idx_wa_msg_ts ON wa_messages(account_id, chat_id, timestamp);
             CREATE INDEX IF NOT EXISTS idx_wa_msg_body ON wa_messages(account_id, body);",
        )
        .context("init whatsapp_data schema")?;
        log::debug!("[whatsapp_data] schema ready");
        Ok(())
    }

    fn open_conn(&self) -> Result<Connection> {
        Connection::open(&self.db_path)
            .with_context(|| format!("open whatsapp_data db: {}", self.db_path.display()))
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Upsert chat metadata rows.  Returns the number of rows inserted or updated.
    pub fn upsert_chats(
        &self,
        account_id: &str,
        chats: &HashMap<String, ChatMeta>,
    ) -> Result<usize> {
        if chats.is_empty() {
            return Ok(0);
        }
        let conn = self.open_conn()?;
        let now = Self::now_secs();
        let mut count = 0usize;
        for (chat_id, meta) in chats {
            let name = meta.name.as_deref().unwrap_or("");
            let is_group = chat_id.ends_with("@g.us") as i64;
            conn.execute(
                "INSERT INTO wa_chats (account_id, chat_id, display_name, is_group, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account_id, chat_id) DO UPDATE SET
                     display_name = CASE WHEN excluded.display_name != '' THEN excluded.display_name ELSE display_name END,
                     is_group     = excluded.is_group,
                     updated_at   = excluded.updated_at",
                params![account_id, chat_id, name, is_group, now],
            )
            .with_context(|| format!("upsert wa_chat {chat_id}"))?;
            count += 1;
        }
        log::debug!(
            "[whatsapp_data] upserted {} chats (account redacted)",
            count
        );
        Ok(count)
    }

    /// Upsert message rows. Returns the number of rows inserted or updated.
    pub fn upsert_messages(&self, account_id: &str, msgs: &[IngestMessage]) -> Result<usize> {
        if msgs.is_empty() {
            return Ok(0);
        }
        let conn = self.open_conn()?;
        let now = Self::now_secs();
        let mut count = 0usize;
        for m in msgs {
            if m.message_id.is_empty() || m.chat_id.is_empty() {
                continue;
            }
            // Persist all messages, including non-text ones (stickers, images,
            // system events).  Dropping empty-body rows biases message_count
            // and last_message_ts to text-only messages, making active chats
            // look stale whenever the latest event has no body.
            let body = m.body.as_deref().unwrap_or("");
            let ts = m.timestamp.unwrap_or(0);
            let from_me = m.from_me.unwrap_or(false) as i64;
            conn.execute(
                "INSERT INTO wa_messages
                     (account_id, chat_id, message_id, sender, sender_jid, from_me,
                      body, timestamp, message_type, source, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(account_id, chat_id, message_id) DO UPDATE SET
                     sender       = CASE WHEN excluded.sender != '' THEN excluded.sender ELSE sender END,
                     sender_jid   = COALESCE(excluded.sender_jid, sender_jid),
                     from_me      = excluded.from_me,
                     body         = CASE WHEN excluded.body != '' THEN excluded.body ELSE body END,
                     timestamp    = excluded.timestamp,
                     message_type = COALESCE(excluded.message_type, message_type),
                     source       = excluded.source,
                     ingested_at  = excluded.ingested_at",
                params![
                    account_id,
                    m.chat_id,
                    m.message_id,
                    m.sender.as_deref().unwrap_or(""),
                    m.sender_jid.as_deref(),
                    from_me,
                    body,
                    ts,
                    m.message_type.as_deref(),
                    m.source.as_deref().unwrap_or(""),
                    now,
                ],
            )
            .with_context(|| {
                format!(
                    "upsert wa_message chat={} msg={}",
                    m.chat_id, m.message_id
                )
            })?;
            count += 1;
        }

        // Refresh chat stats after message upsert.
        if count > 0 {
            conn.execute(
                "UPDATE wa_chats
                 SET message_count   = (SELECT COUNT(*) FROM wa_messages
                                        WHERE wa_messages.account_id = wa_chats.account_id
                                          AND wa_messages.chat_id    = wa_chats.chat_id),
                     last_message_ts = COALESCE(
                                         (SELECT MAX(timestamp) FROM wa_messages
                                          WHERE wa_messages.account_id = wa_chats.account_id
                                            AND wa_messages.chat_id    = wa_chats.chat_id),
                                         last_message_ts),
                     updated_at      = ?1
                 WHERE account_id = ?2",
                rusqlite::params![now, account_id],
            )
            .context("refresh wa_chats stats")?;
        }

        log::debug!(
            "[whatsapp_data] upserted {} messages (account redacted)",
            count
        );
        Ok(count)
    }

    /// Delete messages older than `cutoff_ts` (Unix seconds). Returns the count removed.
    ///
    /// After the delete, refreshes `wa_chats.message_count` and
    /// `last_message_ts` for every chat that lost rows, so `list_chats`
    /// returns accurate counts and ordering immediately.
    pub fn prune_old_messages(&self, cutoff_ts: i64) -> Result<u64> {
        let conn = self.open_conn()?;
        let now = Self::now_secs();

        // Collect affected (account_id, chat_id) pairs before deleting.
        let mut stmt = conn.prepare(
            "SELECT DISTINCT account_id, chat_id FROM wa_messages
             WHERE timestamp > 0 AND timestamp < ?1",
        )?;
        let affected: Vec<(String, String)> = stmt
            .query_map(params![cutoff_ts], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()
            .context("collect affected chats for prune")?;

        let changed = conn
            .execute(
                "DELETE FROM wa_messages WHERE timestamp > 0 AND timestamp < ?1",
                params![cutoff_ts],
            )
            .context("prune old wa_messages")?;

        // Refresh aggregate stats for every affected chat so list_chats
        // reflects the post-prune state immediately.
        if changed > 0 {
            for (acct, chat_id) in &affected {
                conn.execute(
                    "UPDATE wa_chats
                     SET message_count   = (SELECT COUNT(*) FROM wa_messages
                                            WHERE account_id = wa_chats.account_id
                                              AND chat_id    = wa_chats.chat_id),
                         last_message_ts = COALESCE(
                                             (SELECT MAX(timestamp) FROM wa_messages
                                              WHERE account_id = wa_chats.account_id
                                                AND chat_id    = wa_chats.chat_id),
                                             last_message_ts),
                         updated_at      = ?3
                     WHERE account_id = ?1 AND chat_id = ?2",
                    params![acct, chat_id, now],
                )
                .with_context(|| format!("refresh chat stats after prune: {chat_id}"))?;
            }
            log::debug!(
                "[whatsapp_data] pruned {} messages (affected {} chats)",
                changed,
                affected.len()
            );
        }
        Ok(changed as u64)
    }

    /// List chats, optionally filtered by account. Ordered by `last_message_ts` DESC.
    pub fn list_chats(&self, req: &ListChatsRequest) -> Result<Vec<WhatsAppChat>> {
        let conn = self.open_conn()?;
        let limit = req.limit.unwrap_or(50) as i64;
        let offset = req.offset.unwrap_or(0) as i64;

        let chats = if let Some(ref acct) = req.account_id {
            let mut stmt = conn.prepare(
                "SELECT account_id, chat_id, display_name, is_group, last_message_ts,
                        message_count, updated_at
                 FROM wa_chats
                 WHERE account_id = ?1
                 ORDER BY last_message_ts DESC
                 LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt
                .query_map(params![acct, limit, offset], map_chat_row)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("list chats (filtered)")?;
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT account_id, chat_id, display_name, is_group, last_message_ts,
                        message_count, updated_at
                 FROM wa_chats
                 ORDER BY last_message_ts DESC
                 LIMIT ?1 OFFSET ?2",
            )?;
            let rows = stmt
                .query_map(params![limit, offset], map_chat_row)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("list chats (all)")?;
            rows
        };
        log::debug!("[whatsapp_data] list_chats returned {} rows", chats.len());
        Ok(chats)
    }

    /// List messages for a chat, with optional time range and pagination.
    pub fn list_messages(&self, req: &ListMessagesRequest) -> Result<Vec<WhatsAppMessage>> {
        let conn = self.open_conn()?;
        let limit = req.limit.unwrap_or(100) as i64;
        let offset = req.offset.unwrap_or(0) as i64;
        let since_ts = req.since_ts.unwrap_or(0);
        let until_ts = req.until_ts.unwrap_or(i64::MAX);

        let msgs = if let Some(ref acct) = req.account_id {
            let mut stmt = conn.prepare(
                "SELECT account_id, chat_id, message_id, sender, sender_jid, from_me,
                        body, timestamp, message_type, source
                 FROM wa_messages
                 WHERE account_id = ?1
                   AND chat_id    = ?2
                   AND timestamp >= ?3
                   AND timestamp <= ?4
                 ORDER BY timestamp ASC
                 LIMIT ?5 OFFSET ?6",
            )?;
            let rows = stmt
                .query_map(
                    params![acct, req.chat_id, since_ts, until_ts, limit, offset],
                    map_message_row,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("list messages (filtered by account)")?;
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT account_id, chat_id, message_id, sender, sender_jid, from_me,
                        body, timestamp, message_type, source
                 FROM wa_messages
                 WHERE chat_id    = ?1
                   AND timestamp >= ?2
                   AND timestamp <= ?3
                 ORDER BY timestamp ASC
                 LIMIT ?4 OFFSET ?5",
            )?;
            let rows = stmt
                .query_map(
                    params![req.chat_id, since_ts, until_ts, limit, offset],
                    map_message_row,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("list messages (all accounts)")?;
            rows
        };
        log::debug!(
            "[whatsapp_data] list_messages returned {} rows (chat/account redacted)",
            msgs.len()
        );
        Ok(msgs)
    }

    /// Full-text search over message bodies (case-insensitive LIKE).
    pub fn search_messages(&self, req: &SearchMessagesRequest) -> Result<Vec<WhatsAppMessage>> {
        if req.query.trim().is_empty() {
            return Ok(vec![]);
        }
        let conn = self.open_conn()?;
        let limit = req.limit.unwrap_or(20) as i64;
        let pattern = format!("%{}%", req.query.replace('%', "\\%").replace('_', "\\_"));

        // Build the query dynamically depending on optional filters.
        // Each branch binds to a local `rows` variable so `stmt` is dropped
        // before the result is returned (fixes E0597 borrow lifetimes).
        let msgs: Vec<WhatsAppMessage> = match (&req.account_id, &req.chat_id) {
            (Some(acct), Some(chat_id)) => {
                let mut stmt = conn.prepare(
                    "SELECT account_id, chat_id, message_id, sender, sender_jid, from_me,
                            body, timestamp, message_type, source
                     FROM wa_messages
                     WHERE account_id = ?1
                       AND chat_id    = ?2
                       AND body LIKE ?3 ESCAPE '\\'
                     ORDER BY timestamp DESC
                     LIMIT ?4",
                )?;
                let rows = stmt
                    .query_map(params![acct, chat_id, pattern, limit], map_message_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("search messages (account+chat)")?;
                rows
            }
            (Some(acct), None) => {
                let mut stmt = conn.prepare(
                    "SELECT account_id, chat_id, message_id, sender, sender_jid, from_me,
                            body, timestamp, message_type, source
                     FROM wa_messages
                     WHERE account_id = ?1
                       AND body LIKE ?2 ESCAPE '\\'
                     ORDER BY timestamp DESC
                     LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(params![acct, pattern, limit], map_message_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("search messages (account)")?;
                rows
            }
            (None, Some(chat_id)) => {
                let mut stmt = conn.prepare(
                    "SELECT account_id, chat_id, message_id, sender, sender_jid, from_me,
                            body, timestamp, message_type, source
                     FROM wa_messages
                     WHERE chat_id = ?1
                       AND body LIKE ?2 ESCAPE '\\'
                     ORDER BY timestamp DESC
                     LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(params![chat_id, pattern, limit], map_message_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("search messages (chat)")?;
                rows
            }
            (None, None) => {
                let mut stmt = conn.prepare(
                    "SELECT account_id, chat_id, message_id, sender, sender_jid, from_me,
                            body, timestamp, message_type, source
                     FROM wa_messages
                     WHERE body LIKE ?1 ESCAPE '\\'
                     ORDER BY timestamp DESC
                     LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![pattern, limit], map_message_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("search messages (all)")?;
                rows
            }
        };
        log::debug!(
            "[whatsapp_data] search_messages returned {} rows (query/account redacted)",
            msgs.len()
        );
        Ok(msgs)
    }
}

fn map_chat_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WhatsAppChat> {
    Ok(WhatsAppChat {
        account_id: row.get(0)?,
        chat_id: row.get(1)?,
        display_name: row.get(2)?,
        is_group: row.get::<_, i64>(3)? != 0,
        last_message_ts: row.get(4)?,
        message_count: row.get::<_, i64>(5)? as u32,
        updated_at: row.get(6)?,
    })
}

fn map_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WhatsAppMessage> {
    Ok(WhatsAppMessage {
        account_id: row.get(0)?,
        chat_id: row.get(1)?,
        message_id: row.get(2)?,
        sender: row.get(3)?,
        sender_jid: row.get(4)?,
        from_me: row.get::<_, i64>(5)? != 0,
        body: row.get(6)?,
        timestamp: row.get(7)?,
        message_type: row.get(8)?,
        source: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_store() -> (WhatsAppDataStore, tempfile::TempDir) {
        let tmp = tempdir().expect("tempdir");
        let store = WhatsAppDataStore::new(tmp.path()).expect("store");
        (store, tmp)
    }

    #[test]
    fn upsert_and_list_chats() {
        let (store, _tmp) = make_store();
        let mut chats = HashMap::new();
        chats.insert(
            "chat1@c.us".to_string(),
            ChatMeta {
                name: Some("Alice".to_string()),
            },
        );
        chats.insert(
            "group1@g.us".to_string(),
            ChatMeta {
                name: Some("My Group".to_string()),
            },
        );
        let count = store.upsert_chats("acct1", &chats).unwrap();
        assert_eq!(count, 2);

        let req = ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        };
        let rows = store.list_chats(&req).unwrap();
        assert_eq!(rows.len(), 2);

        let group = rows.iter().find(|c| c.chat_id == "group1@g.us").unwrap();
        assert!(group.is_group);
        let dm = rows.iter().find(|c| c.chat_id == "chat1@c.us").unwrap();
        assert!(!dm.is_group);
    }

    #[test]
    fn upsert_and_list_messages() {
        let (store, _tmp) = make_store();
        let mut chats = HashMap::new();
        chats.insert(
            "chat1@c.us".to_string(),
            ChatMeta {
                name: Some("Alice".to_string()),
            },
        );
        store.upsert_chats("acct1", &chats).unwrap();

        let msgs = vec![
            IngestMessage {
                message_id: "msg1".to_string(),
                chat_id: "chat1@c.us".to_string(),
                sender: Some("Alice".to_string()),
                sender_jid: None,
                from_me: Some(false),
                body: Some("Hello there".to_string()),
                timestamp: Some(1_700_000_000),
                message_type: Some("chat".to_string()),
                source: Some("cdp-dom".to_string()),
            },
            IngestMessage {
                message_id: "msg2".to_string(),
                chat_id: "chat1@c.us".to_string(),
                sender: Some("me".to_string()),
                sender_jid: None,
                from_me: Some(true),
                body: Some("Hey!".to_string()),
                timestamp: Some(1_700_000_100),
                message_type: Some("chat".to_string()),
                source: Some("cdp-indexeddb".to_string()),
            },
        ];
        let count = store.upsert_messages("acct1", &msgs).unwrap();
        assert_eq!(count, 2);

        let req = ListMessagesRequest {
            chat_id: "chat1@c.us".to_string(),
            account_id: Some("acct1".to_string()),
            since_ts: None,
            until_ts: None,
            limit: None,
            offset: None,
        };
        let rows = store.list_messages(&req).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].body, "Hello there");
        assert_eq!(rows[1].body, "Hey!");
    }

    #[test]
    fn search_messages_finds_match() {
        let (store, _tmp) = make_store();
        let mut chats = HashMap::new();
        chats.insert(
            "chat1@c.us".to_string(),
            ChatMeta {
                name: Some("Alice".to_string()),
            },
        );
        store.upsert_chats("acct1", &chats).unwrap();

        let msgs = vec![
            IngestMessage {
                message_id: "m1".to_string(),
                chat_id: "chat1@c.us".to_string(),
                sender: Some("Alice".to_string()),
                sender_jid: None,
                from_me: Some(false),
                body: Some("Can you bring the umbrella?".to_string()),
                timestamp: Some(1_700_000_000),
                message_type: None,
                source: Some("cdp-dom".to_string()),
            },
            IngestMessage {
                message_id: "m2".to_string(),
                chat_id: "chat1@c.us".to_string(),
                sender: Some("me".to_string()),
                sender_jid: None,
                from_me: Some(true),
                body: Some("Sure, no problem".to_string()),
                timestamp: Some(1_700_000_200),
                message_type: None,
                source: Some("cdp-dom".to_string()),
            },
        ];
        store.upsert_messages("acct1", &msgs).unwrap();

        let req = SearchMessagesRequest {
            query: "umbrella".to_string(),
            chat_id: None,
            account_id: None,
            limit: None,
        };
        let results = store.search_messages(&req).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].body.contains("umbrella"));
    }

    #[test]
    fn prune_removes_old_messages() {
        let (store, _tmp) = make_store();
        let mut chats = HashMap::new();
        chats.insert("chat1@c.us".to_string(), ChatMeta { name: None });
        store.upsert_chats("acct1", &chats).unwrap();

        let msgs = vec![
            IngestMessage {
                message_id: "old".to_string(),
                chat_id: "chat1@c.us".to_string(),
                sender: None,
                sender_jid: None,
                from_me: Some(false),
                body: Some("Old message".to_string()),
                timestamp: Some(1_000_000),
                message_type: None,
                source: None,
            },
            IngestMessage {
                message_id: "new".to_string(),
                chat_id: "chat1@c.us".to_string(),
                sender: None,
                sender_jid: None,
                from_me: Some(false),
                body: Some("New message".to_string()),
                timestamp: Some(2_000_000_000),
                message_type: None,
                source: None,
            },
        ];
        store.upsert_messages("acct1", &msgs).unwrap();

        let pruned = store.prune_old_messages(1_500_000_000).unwrap();
        assert_eq!(pruned, 1);

        let req = ListMessagesRequest {
            chat_id: "chat1@c.us".to_string(),
            account_id: None,
            since_ts: None,
            until_ts: None,
            limit: None,
            offset: None,
        };
        let remaining = store.list_messages(&req).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].message_id, "new");
    }
}
