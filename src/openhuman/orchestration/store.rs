//! SQLite persistence for the orchestration domain.
//!
//! Lives at `<workspace>/orchestration/orchestration.db`. Message bodies are
//! decrypted plaintext, so this path is workspace-internal (protected by
//! `is_workspace_internal_path`). Follows the subconscious/cron `with_connection`
//! pattern.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::types::{OrchestrationMessage, OrchestrationSession};

const SCHEMA_DDL: &str = "
    PRAGMA foreign_keys = ON;

    -- `status_state`/`current_detail`/`active_call_id` carry the v2 harness
    -- run-state (`status.state`/`status.detail`/`status.active_call_id`). Nullable
    -- and additive: a v1/legacy store gets them via `migrate` (existing rows NULL).
    -- `title`/`model`/`handle`/`repo`/`branch`/`capabilities` carry the v2
    -- `session_info` enrichment (`capabilities` is a JSON array of kind strings).
    -- Also nullable/additive — `migrate` backfills them on an older store.
    CREATE TABLE IF NOT EXISTS sessions (
        session_id      TEXT NOT NULL,
        agent_id        TEXT NOT NULL,
        source          TEXT NOT NULL,
        label           TEXT,
        workspace       TEXT,
        last_seq        INTEGER NOT NULL DEFAULT 0,
        created_at      TEXT NOT NULL,
        last_message_at TEXT NOT NULL,
        status_state    TEXT,
        current_detail  TEXT,
        active_call_id  TEXT,
        title           TEXT,
        model           TEXT,
        handle          TEXT,
        repo            TEXT,
        branch          TEXT,
        capabilities    TEXT,
        PRIMARY KEY (agent_id, session_id)
    );

    -- `event_kind`/`tool_name`/`call_id` carry the v2 per-message event shape
    -- (`event.kind` + tool identity/correlation). `ok`/`is_error`/`exit_code`
    -- carry the `tool_result` outcome. Nullable and additive; v1 and pinned
    -- master/subconscious rows leave them NULL.
    CREATE TABLE IF NOT EXISTS messages (
        id         TEXT PRIMARY KEY,
        agent_id   TEXT NOT NULL,
        session_id TEXT NOT NULL,
        chat_kind  TEXT NOT NULL,
        role       TEXT NOT NULL,
        body       TEXT NOT NULL,
        timestamp  TEXT NOT NULL,
        seq        INTEGER NOT NULL DEFAULT 0,
        event_kind TEXT,
        tool_name  TEXT,
        call_id    TEXT,
        ok         INTEGER,
        is_error   INTEGER,
        exit_code  INTEGER
    );

    CREATE INDEX IF NOT EXISTS idx_messages_session
        ON messages (agent_id, session_id, timestamp);

    CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);

    -- Hosted-brain era: device-side world-observation buffer. Compact notes the
    -- device derives from locally-observed events (inbound DMs, executed effects);
    -- the periodic uploader drains these to `POST /orchestration/v1/world-diff`,
    -- whose receipt schedules the hosted subconscious steering tick. `seq` is a
    -- global monotonic ordinal (unique within any one session — all the backend
    -- dedupe on `(userId, sessionId, seq)` needs). Rows are deleted once uploaded.
    CREATE TABLE IF NOT EXISTS world_obs (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        note       TEXT NOT NULL,
        ts         INTEGER NOT NULL
    );
";

/// Open the orchestration DB, initialise the schema, and run `f`.
pub fn with_connection<T>(
    workspace_dir: &Path,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = workspace_dir.join("orchestration").join("orchestration.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create orchestration dir: {}", parent.display()))?;
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("open orchestration DB: {}", db_path.display()))?;
    // Concurrent writers (the drain ingesting an inbound DM vs the graph's
    // `send_dm` persisting a reply) each open their own connection. Wait for a
    // held write lock instead of erroring `SQLITE_BUSY`; paired with the
    // IMMEDIATE txn in the seq-allocating writers this serialises
    // `MAX(seq)+1 → INSERT` so `seq` stays unique per `(agent_id, session_id)`.
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .context("set orchestration busy_timeout")?;
    // WAL lets readers run concurrently with the single writer, so the one-time,
    // schema-modifying `migrate()` ALTERs (first open after an upgrade) can't be
    // starved into `SQLITE_BUSY` by the drain/`send_dm` writers this store is
    // explicitly shared between — a rollback-journal `ALTER TABLE` needs an
    // EXCLUSIVE lock that any concurrent reader blocks, and a `busy_timeout`
    // expiry there surfaces as the opaque "migrate orchestration schema" failure.
    // `query_row` because `PRAGMA journal_mode` returns the resulting mode.
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
        .context("set orchestration journal_mode=WAL")?;
    conn.execute_batch(SCHEMA_DDL)
        .context("initialise orchestration schema")?;
    migrate(&conn).context("migrate orchestration schema")?;
    f(&conn)
}

/// Run `f` inside a single `BEGIN IMMEDIATE` transaction, rolling back on error.
/// Use for read-then-write allocations (`MAX(seq)+1` then `INSERT`) so two
/// concurrent writers on the same `(agent_id, session_id)` cannot read the same
/// max and persist a duplicate `seq` (which would break the monotonic wake
/// cursor). `IMMEDIATE` takes the write lock up front; the `busy_timeout` set in
/// [`with_connection`] makes the loser wait for the holder to commit rather than
/// fail.
pub fn in_immediate_txn<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .context("begin orchestration immediate txn")?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("COMMIT")
                .context("commit orchestration txn")?;
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// True if `column` already exists on `table` (via `PRAGMA table_info`). Used to
/// make additive `ALTER TABLE ... ADD COLUMN` migrations idempotent — SQLite has
/// no `ADD COLUMN IF NOT EXISTS`, and re-adding an existing column errors.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    // `table` is a hardcoded internal literal (never user input); PRAGMA cannot be
    // parameterised, so it is interpolated directly.
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Additively add `column` to `table` when it is not already present. Idempotent,
/// so it is safe on a fresh DB (SCHEMA_DDL already created the column → no-op) and
/// on an older store (adds it; existing rows default NULL).
fn add_column_if_missing(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    if !column_exists(conn, table, column)? {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
    }
    Ok(())
}

/// One-time, `user_version`-gated migrations. Runs after the idempotent
/// `SCHEMA_DDL`; each version block executes exactly once per DB.
fn migrate(conn: &Connection) -> Result<()> {
    // The v1 world-diff uniqueness migration was retired with the local wake
    // graph — the `world_diff` table no longer exists on fresh stores. Any legacy
    // rows in an upgraded store are harmless; nothing reads them.

    // v2 — additive harness-session-v2 receiver columns. Guarded per-column by a
    // `table_info` existence check rather than `user_version`, so it is order- and
    // freshness-independent: a fresh DB already has them from SCHEMA_DDL (no-op),
    // while a v1 store gains them here with existing rows defaulting NULL — which
    // `derive_status` reads as "no persisted run-state" and falls back to recency.
    add_column_if_missing(conn, "sessions", "status_state", "TEXT")?;
    add_column_if_missing(conn, "sessions", "current_detail", "TEXT")?;
    add_column_if_missing(conn, "sessions", "active_call_id", "TEXT")?;
    add_column_if_missing(conn, "messages", "event_kind", "TEXT")?;
    add_column_if_missing(conn, "messages", "tool_name", "TEXT")?;
    add_column_if_missing(conn, "messages", "call_id", "TEXT")?;
    // v2 tool_result outcome — additive, existing rows default NULL.
    add_column_if_missing(conn, "messages", "ok", "INTEGER")?;
    add_column_if_missing(conn, "messages", "is_error", "INTEGER")?;
    add_column_if_missing(conn, "messages", "exit_code", "INTEGER")?;

    // v2 `session_info` enrichment columns (spec §4). Same per-column,
    // freshness-independent guard as the run-state block above: a fresh DB has
    // them from SCHEMA_DDL (no-op); a pre-session_info store gains them here with
    // existing rows defaulting NULL. `capabilities` holds a JSON array of kinds.
    add_column_if_missing(conn, "sessions", "title", "TEXT")?;
    add_column_if_missing(conn, "sessions", "model", "TEXT")?;
    add_column_if_missing(conn, "sessions", "handle", "TEXT")?;
    add_column_if_missing(conn, "sessions", "repo", "TEXT")?;
    add_column_if_missing(conn, "sessions", "branch", "TEXT")?;
    add_column_if_missing(conn, "sessions", "capabilities", "TEXT")?;
    Ok(())
}

/// True if a relay message id is already persisted. This guard MUST run before
/// decryption so the non-idempotent Signal double-ratchet is never advanced
/// twice for the same message.
pub fn message_exists(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM messages WHERE id = ?1", params![id], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

/// Insert or update the session row (keyed by agent + session). The
/// `session_info` enrichment columns COALESCE like the run-state ones, so an
/// ordinary event (which carries none) never wipes a prior intro's metadata, and
/// a later `session_info` (`resumed=true`) refreshes rather than duplicates.
/// `capabilities` is stored as a JSON array; an empty list encodes to NULL so it
/// COALESCEs to "no change" instead of clobbering a prior non-empty list.
pub fn upsert_session(conn: &Connection, s: &OrchestrationSession) -> Result<()> {
    let capabilities = encode_capabilities(&s.capabilities);
    conn.execute(
        "INSERT INTO sessions
           (session_id, agent_id, source, label, workspace, last_seq, created_at, last_message_at,
            status_state, current_detail, active_call_id,
            title, model, handle, repo, branch, capabilities)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(agent_id, session_id) DO UPDATE SET
           last_seq = MAX(sessions.last_seq, excluded.last_seq),
           -- Keep the newest activity time: a read-sync pulling an older hosted
           -- `lastEventTs` must not regress a just-rendered local reply. rfc3339
           -- (consistent format) compares lexicographically.
           last_message_at = MAX(sessions.last_message_at, excluded.last_message_at),
           label = COALESCE(excluded.label, sessions.label),
           workspace = COALESCE(excluded.workspace, sessions.workspace),
           -- Run-state fields COALESCE so a content event (which carries none)
           -- never wipes the last status; a fresh `status` event overwrites them.
           status_state = COALESCE(excluded.status_state, sessions.status_state),
           current_detail = COALESCE(excluded.current_detail, sessions.current_detail),
           active_call_id = COALESCE(excluded.active_call_id, sessions.active_call_id),
           -- session_info enrichment: COALESCE so non-session_info events preserve
           -- the last intro's metadata, and a `resumed=true` re-intro refreshes it.
           title = COALESCE(excluded.title, sessions.title),
           model = COALESCE(excluded.model, sessions.model),
           handle = COALESCE(excluded.handle, sessions.handle),
           repo = COALESCE(excluded.repo, sessions.repo),
           branch = COALESCE(excluded.branch, sessions.branch),
           capabilities = COALESCE(excluded.capabilities, sessions.capabilities)",
        params![
            s.session_id,
            s.agent_id,
            s.source,
            s.label,
            s.workspace,
            s.last_seq,
            s.created_at,
            s.last_message_at,
            s.status_state,
            s.current_detail,
            s.active_call_id,
            s.title,
            s.model,
            s.handle,
            s.repo,
            s.branch,
            capabilities,
        ],
    )?;
    Ok(())
}

/// Encode `session_info.capabilities` for the `sessions.capabilities` TEXT column:
/// a JSON array, or `None` for an empty list so the COALESCE upsert treats it as
/// "no update" (a content/status event carries no capabilities and must not wipe
/// a prior intro's list).
fn encode_capabilities(capabilities: &[String]) -> Option<String> {
    if capabilities.is_empty() {
        return None;
    }
    // A `Vec<String>` always serialises, but fall back to NULL rather than
    // failing the whole upsert on the impossible error path.
    serde_json::to_string(capabilities).ok()
}

/// Decode the `sessions.capabilities` JSON array back into a `Vec<String>`. A
/// NULL/absent or malformed value reads as an empty list (never an error).
fn decode_capabilities(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Overwrite a session's v2 run-state columns from an authoritative `status`
/// snapshot. `upsert_session` COALESCEs these so a content event (which carries
/// no run-state) never wipes the last status; a `status` event, by contrast, OWNS
/// them and must be able to CLEAR `current_detail`/`active_call_id` on a
/// `running_tool` → `idle` transition. The row already exists (the ingest path
/// runs `upsert_session` first), so this is a plain UPDATE that SETs — not
/// coalesces — all three, letting `None` clear a stale value.
pub fn apply_run_state(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
    status_state: Option<&str>,
    current_detail: Option<&str>,
    active_call_id: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE sessions
            SET status_state = ?3, current_detail = ?4, active_call_id = ?5
          WHERE agent_id = ?1 AND session_id = ?2",
        params![
            agent_id,
            session_id,
            status_state,
            current_detail,
            active_call_id
        ],
    )?;
    Ok(())
}

/// Insert a message, idempotent by relay id. Returns true if a new row landed.
pub fn insert_message(conn: &Connection, m: &OrchestrationMessage) -> Result<bool> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO messages
           (id, agent_id, session_id, chat_kind, role, body, timestamp, seq,
            event_kind, tool_name, call_id, ok, is_error, exit_code)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            m.id,
            m.agent_id,
            m.session_id,
            m.chat_kind.as_str(),
            m.role,
            m.body,
            m.timestamp,
            m.seq,
            m.event_kind,
            m.tool_name,
            m.call_id,
            m.ok,
            m.is_error,
            m.exit_code,
        ],
    )?;
    Ok(changed > 0)
}

/// Count persisted messages for a session (test/observability helper).
pub fn count_messages(conn: &Connection, agent_id: &str, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE agent_id = ?1 AND session_id = ?2",
        params![agent_id, session_id],
        |row| row.get(0),
    )?)
}

/// Count transcript-visible messages for a session, using the same visibility
/// predicate as message reads, unread counts, and roster previews.
pub fn count_visible_messages(conn: &Connection, agent_id: &str, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE agent_id = ?1 AND session_id = ?2
             AND (event_kind IS NULL
                  OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info'))",
        params![agent_id, session_id],
        |row| row.get(0),
    )?)
}

/// Count transcript-visible messages for a pinned chat, whose transcript is
/// scoped only by `session_id` and can include rows from multiple peers.
pub fn count_visible_messages_by_session(conn: &Connection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE session_id = ?1
             AND (event_kind IS NULL
                  OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info'))",
        params![session_id],
        |row| row.get(0),
    )?)
}

/// Transcript-visible message counts keyed by `(agent_id, session_id)`.
pub fn visible_message_counts(conn: &Connection) -> Result<HashMap<(String, String), i64>> {
    let mut stmt = conn.prepare(
        "SELECT agent_id, session_id, COUNT(*)
           FROM messages
          WHERE event_kind IS NULL
             OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info')
          GROUP BY agent_id, session_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// Transcript-visible message counts keyed by `session_id` for pinned chats.
pub fn visible_message_counts_by_session(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, COUNT(*)
           FROM messages
          WHERE event_kind IS NULL
             OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info')
          GROUP BY session_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// The next monotonic per-session ingest ordinal: `MAX(seq) + 1` over the
/// session's messages (`1` for the first message). Stamped at persist time so
/// the wake idempotence cursor rides a strictly-increasing value instead of the
/// harness `message.line`, which is unreliable (a wrapped Claude harness stamps
/// `line = 0` on every DM, and a peer can reuse/reset it across harness sessions
/// under one shared `wrapper_session_id`). Messages are append-only and
/// deduped-by-id before persist, so this is strictly increasing. (#4583)
pub fn next_session_seq(conn: &Connection, agent_id: &str, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE agent_id = ?1 AND session_id = ?2",
        params![agent_id, session_id],
        |row| row.get(0),
    )?)
}

/// The current `last_seq` for a session, or `None` if the session row does not
/// exist yet. Used to detect a non-monotonic inbound `seq` before the upsert
/// clamps it away via `MAX(...)`.
pub fn session_last_seq(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT last_seq FROM sessions WHERE agent_id = ?1 AND session_id = ?2",
            params![agent_id, session_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// The most recent message body for a session — the roster task line's "current
/// activity". Newest by timestamp then seq; `None` when the session has no
/// messages yet. Body is decrypted plaintext (workspace-internal, like the rest
/// of this store).
pub fn latest_message_preview(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT body FROM messages
               WHERE agent_id = ?1 AND session_id = ?2
                 AND (event_kind IS NULL
                      OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info'))
               ORDER BY timestamp DESC, seq DESC LIMIT 1",
            params![agent_id, session_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// List every persisted session row, newest activity first (stage-7 read surface).
pub fn list_sessions(conn: &Connection) -> Result<Vec<OrchestrationSession>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, agent_id, source, label, workspace, last_seq, created_at, last_message_at,
                status_state, current_detail, active_call_id,
                title, model, handle, repo, branch, capabilities
           FROM sessions ORDER BY last_message_at DESC",
    )?;
    let rows = stmt
        .query_map([], map_session_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// List messages for a chat keyed by `session_id` alone (so the pinned `master` /
/// `subconscious` windows aggregate across peers). Newest `limit` returned in
/// chronological order; `before` (exclusive timestamp) pages backwards.
pub fn list_messages_by_session(
    conn: &Connection,
    session_id: &str,
    limit: u32,
    before: Option<&str>,
) -> Result<Vec<OrchestrationMessage>> {
    let rows = match before {
        Some(before) => {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, session_id, chat_kind, role, body, timestamp, seq,
                        event_kind, tool_name, call_id, ok, is_error, exit_code
                   FROM messages WHERE session_id = ?1 AND timestamp < ?2
                     AND (event_kind IS NULL
                          OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info'))
                   ORDER BY timestamp DESC, seq DESC LIMIT ?3",
            )?;
            let rows = stmt
                .query_map(params![session_id, before, limit], map_message_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, session_id, chat_kind, role, body, timestamp, seq,
                        event_kind, tool_name, call_id, ok, is_error, exit_code
                   FROM messages WHERE session_id = ?1
                     AND (event_kind IS NULL
                          OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info'))
                   ORDER BY timestamp DESC, seq DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![session_id, limit], map_message_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        }
    };
    Ok(rows.into_iter().rev().collect())
}

/// The newest non-bookkeeping message for a specific `(agent_id, session_id)` — the
/// agent-scoped analogue of [`list_messages_by_session`]'s newest row (same
/// `status`/`lifecycle`/`unknown`/`session_info` exclusion). Used by the one-shot
/// history migration: `list_messages_by_session` reads by `session_id` alone, so a
/// legacy session id shared with another peer could surface *that* peer's latest
/// turn and replay the wrong ask under this session's `agent_id`.
pub fn latest_content_message(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
) -> Result<Option<OrchestrationMessage>> {
    conn.query_row(
        "SELECT id, agent_id, session_id, chat_kind, role, body, timestamp, seq,
                event_kind, tool_name, call_id, ok, is_error, exit_code
           FROM messages WHERE agent_id = ?1 AND session_id = ?2
             AND (event_kind IS NULL
                  OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info'))
           ORDER BY timestamp DESC, seq DESC LIMIT 1",
        params![agent_id, session_id],
        map_message_row,
    )
    .optional()
    .map_err(Into::into)
}

/// Row → [`OrchestrationMessage`] mapper (a free fn so it is `Copy` and can be
/// reused across the two `query_map` arms without a borrow-lifetime tangle).
/// Column order MUST match the `SELECT` lists in the message readers.
fn map_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrchestrationMessage> {
    let chat_kind: String = row.get(3)?;
    Ok(OrchestrationMessage {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        chat_kind: crate::openhuman::orchestration::types::ChatKind::from_str(&chat_kind),
        role: row.get(4)?,
        body: row.get(5)?,
        timestamp: row.get(6)?,
        seq: row.get(7)?,
        event_kind: row.get(8)?,
        tool_name: row.get(9)?,
        call_id: row.get(10)?,
        ok: row.get(11)?,
        is_error: row.get(12)?,
        exit_code: row.get(13)?,
    })
}

/// Row → [`OrchestrationSession`] mapper. Column order MUST match the `SELECT`
/// lists in [`list_sessions`] and [`load_session`].
fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrchestrationSession> {
    Ok(OrchestrationSession {
        session_id: row.get(0)?,
        agent_id: row.get(1)?,
        source: row.get(2)?,
        label: row.get(3)?,
        workspace: row.get(4)?,
        last_seq: row.get(5)?,
        created_at: row.get(6)?,
        last_message_at: row.get(7)?,
        status_state: row.get(8)?,
        current_detail: row.get(9)?,
        active_call_id: row.get(10)?,
        title: row.get(11)?,
        model: row.get(12)?,
        handle: row.get(13)?,
        repo: row.get(14)?,
        branch: row.get(15)?,
        capabilities: decode_capabilities(row.get(16)?),
    })
}

/// Count unread messages for a chat: rows with `timestamp` after the read cursor.
pub fn unread_count(conn: &Connection, session_id: &str) -> Result<i64> {
    let cursor = kv_get(conn, &read_cursor_key(session_id))?.unwrap_or_default();
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND timestamp > ?2
             AND (event_kind IS NULL
                  OR event_kind NOT IN ('status', 'lifecycle', 'unknown', 'session_info'))",
        params![session_id, cursor],
        |row| row.get(0),
    )?)
}

/// Advance a chat's read cursor to its newest message timestamp (mark-read).
pub fn mark_chat_read(conn: &Connection, session_id: &str) -> Result<()> {
    let latest: Option<String> = conn
        .query_row(
            "SELECT MAX(timestamp) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(latest) = latest {
        kv_set(conn, &read_cursor_key(session_id), &latest)?;
    }
    Ok(())
}

/// The agent_id of the most recent `master`-window message — the default
/// recipient when the human sends a Master steering DM.
pub fn latest_master_peer(conn: &Connection) -> Result<Option<String>> {
    conn.query_row(
        "SELECT agent_id FROM messages WHERE session_id = 'master'
           ORDER BY timestamp DESC, seq DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// The contact (`agent_id`) that owns a given session id, if the session exists.
pub fn session_agent_id(conn: &Connection, session_id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT agent_id FROM sessions WHERE session_id = ?1 LIMIT 1",
        params![session_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// The most recent non-pinned session id for a peer agent, if any — the thread to
/// reuse when OpenHuman initiates an outbound ask to that peer, so the peer's
/// reply threads back into the same session (shared `wrapper_session_id` model,
/// #227/#4582). Newest by `last_message_at`. Returns `None` when there is no
/// existing thread with the peer (caller mints a fresh session id).
pub fn latest_session_for_agent(conn: &Connection, agent_id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT session_id FROM sessions
           WHERE agent_id = ?1 AND session_id NOT IN ('master', 'subconscious')
           ORDER BY last_message_at DESC LIMIT 1",
        params![agent_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn read_cursor_key(session_id: &str) -> String {
    format!("read:{session_id}")
}

/// Ingest-cursor lag (stage-8 health): how many sessions have a latest message
/// seq beyond the wake-cursor seq already processed — i.e. pending wake work. A
/// persistently non-zero value signals the wake loop is stuck.
pub fn ingest_cursor_lag(conn: &Connection) -> Result<i64> {
    let mut stmt = conn.prepare("SELECT agent_id, session_id, last_seq FROM sessions")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut lag = 0i64;
    for (agent_id, session_id, last_seq) in rows {
        // Master/subconscious windows are UI-only, not wake-driven — skip them.
        if session_id == "master" || session_id == "subconscious" {
            continue;
        }
        let cursor = kv_get(conn, &format!("cursor:{agent_id}:{session_id}"))?
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(i64::MIN);
        if last_seq > cursor {
            lag += 1;
        }
    }
    Ok(lag)
}

/// Load a single session row (the session's counterpart + metadata).
pub fn load_session(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
) -> Result<Option<OrchestrationSession>> {
    conn.query_row(
        "SELECT session_id, agent_id, source, label, workspace, last_seq, created_at, last_message_at,
                status_state, current_detail, active_call_id,
                title, model, handle, repo, branch, capabilities
           FROM sessions WHERE agent_id = ?1 AND session_id = ?2",
        params![agent_id, session_id],
        map_session_row,
    )
    .optional()
    .map_err(Into::into)
}

/// Load the most recent `limit` messages for a session, returned in chronological
/// (oldest-first) order so the graph reads them like a transcript.
pub fn list_recent_messages(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
    limit: u32,
) -> Result<Vec<OrchestrationMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, session_id, chat_kind, role, body, timestamp, seq,
                event_kind, tool_name, call_id, ok, is_error, exit_code
           FROM messages WHERE agent_id = ?1 AND session_id = ?2
           ORDER BY timestamp DESC, seq DESC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![agent_id, session_id, limit], map_message_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Reverse the DESC scan back to chronological order.
    Ok(rows.into_iter().rev().collect())
}

// ── Device world-observation buffer (hosted-brain uploader) ──────────────────

/// One buffered world-observation row awaiting upload to the hosted brain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldObs {
    pub id: i64,
    pub session_id: String,
    pub seq: i64,
    pub note: String,
    pub ts: i64,
}

/// Append one device-observed world note. `seq` is a global monotonic ordinal
/// (unique within any single session, which is all the backend dedupe on
/// `(userId, sessionId, seq)` requires). Returns the assigned seq.
/// Buffer a world observation for the periodic world-diff uploader.
///
/// `seq` is drawn from the **persistent, monotonic** `world_obs:next_seq` counter in
/// `kv` — it must never reset. The backend dedupes world-diff entries on
/// `(userId, sessionId, seq)`, and the uploader deletes every row after a successful
/// push ([`delete_world_obs`]), so a `MAX(seq)+1`-over-the-table scheme restarts at
/// `1` once the buffer drains and reuses seqs the backend already saw — it then
/// treats the fresh observation as a duplicate `202` no-op and silently drops it, so
/// only the first batch per session ever reaches the hosted subconscious tier. The
/// counter is seeded above any still-buffered row so an upgraded DB with pending rows
/// can't collide either, and the allocate-then-insert runs in an immediate txn so two
/// concurrent appends can't draw the same ordinal.
pub fn append_world_obs(conn: &Connection, session_id: &str, note: &str, ts: i64) -> Result<i64> {
    in_immediate_txn(conn, |conn| {
        let kv_max: i64 = kv_get(conn, "world_obs:next_seq")?
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        let table_max: i64 =
            conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM world_obs", [], |r| {
                r.get(0)
            })?;
        let next_seq = kv_max.max(table_max) + 1;
        kv_set(conn, "world_obs:next_seq", &next_seq.to_string())?;
        conn.execute(
            "INSERT INTO world_obs (session_id, seq, note, ts) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, next_seq, note, ts],
        )?;
        Ok(next_seq)
    })
}

/// Drain up to `limit` oldest buffered world observations (FIFO by insert order).
/// Rows stay until [`delete_world_obs`] confirms a successful upload, so a failed
/// flush retries on the next tick.
pub fn drain_world_obs(conn: &Connection, limit: u32) -> Result<Vec<WorldObs>> {
    let mut stmt = conn
        .prepare("SELECT id, session_id, seq, note, ts FROM world_obs ORDER BY id ASC LIMIT ?1")?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(WorldObs {
                id: r.get(0)?,
                session_id: r.get(1)?,
                seq: r.get(2)?,
                note: r.get(3)?,
                ts: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete buffered world observations by id after a confirmed upload.
pub fn delete_world_obs(conn: &Connection, ids: &[i64]) -> Result<()> {
    for id in ids {
        conn.execute("DELETE FROM world_obs WHERE id = ?1", params![id])?;
    }
    Ok(())
}

/// Read a `kv` value (used for the per-session idempotence cursor).
pub fn kv_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT v FROM kv WHERE k = ?1", params![key], |r| r.get(0))
        .optional()
        .map_err(Into::into)
}

/// Write a `kv` value (upsert).
pub fn kv_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO kv (k, v) VALUES (?1, ?2)
           ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![key, value],
    )?;
    Ok(())
}

/// Delete a `kv` value (no-op if absent).
pub fn kv_delete(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM kv WHERE k = ?1", params![key])?;
    Ok(())
}

// ── Outbound-ask correlation (Master chat, W7) ───────────────────────────────
//
// When OpenHuman DMs a peer on the user's behalf (`orchestration_send_to_agent`),
// we record a ONE-SHOT pending ask keyed by the outbound session id, mapping it
// to the window the ask originated from (usually `master`). When the peer's reply
// lands under that session id (shared `wrapper_session_id`), the wake path threads
// the answer back into the origin window instead of auto-replying to the peer.
//
// This is a pragmatic 1:1 request/response correlation: it assumes the next
// inbound message on the ask session is the answer. A robust many-in-flight
// correlation needs an explicit envelope `inReplyTo` (tracked as F3 / #4583's
// follow-ups); until then this covers the common single-ask case.

/// Scope the pending-ask key by BOTH the answering peer and the session id.
/// Sessions/checkpoints are keyed by `(agent, session)`, and legacy wrapper
/// session ids can collide across peers (see the F2 checkpoint fix); keying by
/// session id alone would let a *different* peer's inbound on a shared legacy
/// session id consume the ask and misroute the reply.
fn pending_ask_key(peer_agent_id: &str, ask_session_id: &str) -> String {
    format!("pending_ask:{peer_agent_id}:{ask_session_id}")
}

/// Record a one-shot pending outbound ask: `(peer_agent_id, ask_session_id)` →
/// `origin_session_id`.
pub fn set_pending_ask(
    conn: &Connection,
    peer_agent_id: &str,
    ask_session_id: &str,
    origin_session_id: &str,
) -> Result<()> {
    kv_set(
        conn,
        &pending_ask_key(peer_agent_id, ask_session_id),
        origin_session_id,
    )
}

/// The origin window for a pending ask on `(peer_agent_id, ask_session_id)`.
pub fn pending_ask_origin(
    conn: &Connection,
    peer_agent_id: &str,
    ask_session_id: &str,
) -> Result<Option<String>> {
    kv_get(conn, &pending_ask_key(peer_agent_id, ask_session_id))
}

/// Clear a pending ask once its answer has been threaded back (one-shot).
pub fn clear_pending_ask(
    conn: &Connection,
    peer_agent_id: &str,
    ask_session_id: &str,
) -> Result<()> {
    kv_delete(conn, &pending_ask_key(peer_agent_id, ask_session_id))
}

#[cfg(test)]
mod tests {
    use super::super::types::ChatKind;
    use super::*;

    fn msg(id: &str, agent: &str, session: &str, seq: i64) -> OrchestrationMessage {
        OrchestrationMessage {
            id: id.into(),
            agent_id: agent.into(),
            session_id: session.into(),
            chat_kind: ChatKind::Session,
            role: "agent".into(),
            body: "hi".into(),
            timestamp: "2026-07-02T00:00:00Z".into(),
            seq,
            ..Default::default()
        }
    }

    fn session(agent: &str, session: &str, seq: i64) -> OrchestrationSession {
        OrchestrationSession {
            session_id: session.into(),
            agent_id: agent.into(),
            source: "claude".into(),
            label: None,
            workspace: None,
            last_seq: seq,
            created_at: "2026-07-02T00:00:00Z".into(),
            last_message_at: "2026-07-02T00:00:00Z".into(),
            ..Default::default()
        }
    }

    #[test]
    fn persists_and_dedupes_by_message_id() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            upsert_session(conn, &session("@a", "h1", 1))?;
            assert!(!message_exists(conn, "m1")?);
            assert!(insert_message(conn, &msg("m1", "@a", "h1", 1))?);
            // Replay of the same id is a no-op and stays deduped.
            assert!(!insert_message(conn, &msg("m1", "@a", "h1", 1))?);
            assert!(message_exists(conn, "m1")?);
            assert_eq!(count_messages(conn, "@a", "h1")?, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn persists_and_reads_back_tool_result_outcome() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            upsert_session(conn, &session("@a", "h1", 1))?;
            let failed = OrchestrationMessage {
                event_kind: Some("tool_result".into()),
                tool_name: Some("Bash".into()),
                call_id: Some("c1".into()),
                ok: Some(false),
                is_error: Some(true),
                exit_code: Some(1),
                ..msg("m1", "@a", "h1", 1)
            };
            assert!(insert_message(conn, &failed)?);
            let back = list_recent_messages(conn, "@a", "h1", 10)?;
            assert_eq!(back.len(), 1);
            assert_eq!(back[0].ok, Some(false));
            assert_eq!(back[0].is_error, Some(true));
            assert_eq!(back[0].exit_code, Some(1));
            // A plain message leaves the outcome columns NULL → None on read.
            assert!(insert_message(conn, &msg("m2", "@a", "h1", 2))?);
            let plain = list_recent_messages(conn, "@a", "h1", 10)?;
            let m2 = plain.iter().find(|m| m.id == "m2").unwrap();
            assert_eq!(m2.ok, None);
            assert_eq!(m2.is_error, None);
            assert_eq!(m2.exit_code, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn latest_message_preview_returns_newest_or_none() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            upsert_session(conn, &session("@a", "h1", 1))?;
            // No messages yet.
            assert_eq!(latest_message_preview(conn, "@a", "h1")?, None);

            // Same timestamp → newest is decided by seq DESC.
            insert_message(conn, &msg("m1", "@a", "h1", 1))?;
            let mut newer = msg("m2", "@a", "h1", 2);
            newer.body = "later line".into();
            insert_message(conn, &newer)?;
            assert_eq!(
                latest_message_preview(conn, "@a", "h1")?.as_deref(),
                Some("later line")
            );

            // Scoped to (agent, session): a different session is not returned.
            assert_eq!(latest_message_preview(conn, "@a", "other")?, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn capabilities_codec_round_trips_and_is_null_safe() {
        // Non-empty → JSON array; decodes back identically.
        let caps = vec!["agent_message".to_string(), "tool_call".to_string()];
        let encoded = encode_capabilities(&caps).expect("non-empty encodes");
        assert_eq!(encoded, r#"["agent_message","tool_call"]"#);
        assert_eq!(decode_capabilities(Some(encoded)), caps);

        // Empty → NULL (so the COALESCE upsert treats it as "no update").
        assert_eq!(encode_capabilities(&[]), None);

        // NULL / malformed decode to an empty list, never an error.
        assert!(decode_capabilities(None).is_empty());
        assert!(decode_capabilities(Some("not json".into())).is_empty());
    }

    #[test]
    fn session_info_enrichment_persists_and_coalesces_across_upserts() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            // First upsert carries the full intro.
            let intro = OrchestrationSession {
                title: Some("Intro".into()),
                model: Some("opus".into()),
                handle: Some("@alice".into()),
                repo: Some("org/myrepo".into()),
                branch: Some("feat/x".into()),
                capabilities: vec!["agent_message".into()],
                ..session("@a", "h1", 0)
            };
            upsert_session(conn, &intro)?;
            let loaded = load_session(conn, "@a", "h1")?.expect("session");
            assert_eq!(loaded.title.as_deref(), Some("Intro"));
            assert_eq!(loaded.capabilities, vec!["agent_message".to_string()]);

            // A subsequent upsert with NO enrichment (e.g. a content event) must
            // COALESCE — the intro metadata survives.
            upsert_session(conn, &session("@a", "h1", 1))?;
            let after = load_session(conn, "@a", "h1")?.expect("session");
            assert_eq!(
                after.title.as_deref(),
                Some("Intro"),
                "title survives a bare upsert"
            );
            assert_eq!(after.model.as_deref(), Some("opus"));
            assert_eq!(after.capabilities, vec!["agent_message".to_string()]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn status_lifecycle_unknown_rows_are_hidden_from_thread_and_unread() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            upsert_session(conn, &session("@a", "h1", 1))?;

            // A v1 row (no event_kind) and typed content rows stay visible;
            // status/lifecycle/unknown/session_info are persisted (for relay dedup)
            // but must not surface in the thread or the unread count.
            let mut plain = msg("v1", "@a", "h1", 1);
            plain.timestamp = "2026-07-02T00:00:01Z".into();
            insert_message(conn, &plain)?;

            let mut call = msg("call", "@a", "h1", 2);
            call.event_kind = Some("tool_call".into());
            call.timestamp = "2026-07-02T00:00:02Z".into();
            insert_message(conn, &call)?;

            for (id, kind, seq) in [
                ("st", "status", 3),
                ("lc", "lifecycle", 4),
                ("uk", "unknown", 5),
                ("si", "session_info", 6),
            ] {
                let mut hidden = msg(id, "@a", "h1", seq);
                hidden.event_kind = Some(kind.into());
                hidden.timestamp = format!("2026-07-02T00:00:0{seq}Z");
                insert_message(conn, &hidden)?;
            }

            let thread = list_messages_by_session(conn, "h1", 50, None)?;
            let ids: Vec<&str> = thread.iter().map(|m| m.id.as_str()).collect();
            assert_eq!(ids, vec!["v1", "call"], "only v1 + typed content rows");

            // Unread (cursor at 0) counts the two visible rows, not the 4 hidden.
            assert_eq!(unread_count(conn, "h1")?, 2);
            // UI session summaries use the same visibility predicate as unread and
            // transcript reads, while the raw observability count still includes
            // all persisted relay-dedupe rows.
            assert_eq!(count_visible_messages(conn, "@a", "h1")?, 2);
            assert_eq!(count_messages(conn, "@a", "h1")?, 6);

            let mut other_peer = msg("other-peer", "@b", "h1", 7);
            other_peer.timestamp = "2026-07-02T00:00:07Z".into();
            insert_message(conn, &other_peer)?;
            assert_eq!(count_visible_messages(conn, "@a", "h1")?, 2);
            assert_eq!(count_visible_messages_by_session(conn, "h1")?, 3);
            let by_agent_session = visible_message_counts(conn)?;
            assert_eq!(
                by_agent_session.get(&("@a".to_string(), "h1".to_string())),
                Some(&2)
            );
            assert_eq!(
                by_agent_session.get(&("@b".to_string(), "h1".to_string())),
                Some(&1)
            );
            let by_session = visible_message_counts_by_session(conn)?;
            assert_eq!(by_session.get("h1"), Some(&3));

            // Roster preview skips the hidden rows → newest visible is the call.
            assert_eq!(
                latest_message_preview(conn, "@a", "h1")?.as_deref(),
                Some("hi"),
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn read_surface_lists_sessions_messages_and_tracks_unread() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            // Two sessions: a harness session and the pinned master window.
            upsert_session(conn, &session("@peer", "h1", 2))?;
            insert_message(conn, &msg("m1", "@peer", "h1", 1))?;
            let mut m2 = msg("m2", "@peer", "h1", 2);
            m2.timestamp = "2026-07-02T00:05:00Z".into();
            insert_message(conn, &m2)?;

            // list_sessions returns the row; messages come back chronologically.
            let sessions = list_sessions(conn)?;
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].session_id, "h1");
            let msgs = list_messages_by_session(conn, "h1", 100, None)?;
            assert_eq!(
                msgs.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                vec!["m1", "m2"]
            );

            // Both messages are unread until we mark the chat read.
            assert_eq!(unread_count(conn, "h1")?, 2);
            mark_chat_read(conn, "h1")?;
            assert_eq!(unread_count(conn, "h1")?, 0);

            // `before` pages backwards (exclusive).
            let older = list_messages_by_session(conn, "h1", 100, Some("2026-07-02T00:05:00Z"))?;
            assert_eq!(older.len(), 1);
            assert_eq!(older[0].id, "m1");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn latest_master_peer_resolves_the_send_recipient() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            assert!(latest_master_peer(conn)?.is_none());
            let mut master = msg("mm", "@owner-agent", "master", 0);
            master.chat_kind = ChatKind::Master;
            insert_message(conn, &master)?;
            assert_eq!(latest_master_peer(conn)?.as_deref(), Some("@owner-agent"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn latest_session_for_agent_reuses_newest_thread_and_ignores_pinned() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            // No thread with the peer yet → caller mints a fresh id.
            assert!(latest_session_for_agent(conn, "@peer")?.is_none());

            // Two threads with the peer; the newest by last_message_at wins.
            let mut old = session("@peer", "s-old", 1);
            old.last_message_at = "2026-07-02T00:01:00Z".into();
            upsert_session(conn, &old)?;
            let mut new = session("@peer", "s-new", 1);
            new.last_message_at = "2026-07-02T00:09:00Z".into();
            upsert_session(conn, &new)?;
            assert_eq!(
                latest_session_for_agent(conn, "@peer")?.as_deref(),
                Some("s-new")
            );

            // A pinned window for the same agent id must never be reused.
            let mut pinned = session("@peer", "master", 1);
            pinned.last_message_at = "2026-07-02T23:00:00Z".into();
            upsert_session(conn, &pinned)?;
            assert_eq!(
                latest_session_for_agent(conn, "@peer")?.as_deref(),
                Some("s-new"),
                "pinned window excluded despite newer timestamp"
            );

            // Scoped by agent: a different peer has no thread.
            assert!(latest_session_for_agent(conn, "@other")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn pending_ask_correlation_is_one_shot() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            // Nothing pending initially.
            assert!(pending_ask_origin(conn, "peer-a", "s-ask")?.is_none());
            // Record an ask to `peer-a` on session `s-ask` from the master window.
            set_pending_ask(conn, "peer-a", "s-ask", "master")?;
            assert_eq!(
                pending_ask_origin(conn, "peer-a", "s-ask")?.as_deref(),
                Some("master")
            );
            // Scoped by (agent, session) — same session id under a DIFFERENT peer
            // must not satisfy the ask (legacy session-id collision guard).
            assert!(pending_ask_origin(conn, "peer-b", "s-ask")?.is_none());
            // A different session under the same peer is also unaffected.
            assert!(pending_ask_origin(conn, "peer-a", "s-other")?.is_none());
            // Clearing consumes it (one-shot).
            clear_pending_ask(conn, "peer-a", "s-ask")?;
            assert!(pending_ask_origin(conn, "peer-a", "s-ask")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn upsert_advances_last_seq_monotonically() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            upsert_session(conn, &session("@a", "h1", 5))?;
            upsert_session(conn, &session("@a", "h1", 2))?; // lower seq must not regress
            let seq: i64 = conn.query_row(
                "SELECT last_seq FROM sessions WHERE agent_id='@a' AND session_id='h1'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(seq, 5);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migrates_pre_v2_schema_by_adding_session_and_message_columns() {
        // A store created before the harness-session-v2 receiver: the sessions and
        // messages tables lack the new run-state / event columns. Opening through
        // `with_connection` must add them additively (existing rows read NULL) and
        // then accept writes that populate them — proving the ALTER path, not just
        // the fresh-DDL path, works.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("orchestration").join("orchestration.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (
                     session_id TEXT NOT NULL, agent_id TEXT NOT NULL, source TEXT NOT NULL,
                     label TEXT, workspace TEXT, last_seq INTEGER NOT NULL DEFAULT 0,
                     created_at TEXT NOT NULL, last_message_at TEXT NOT NULL,
                     PRIMARY KEY (agent_id, session_id));
                 CREATE TABLE messages (
                     id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, session_id TEXT NOT NULL,
                     chat_kind TEXT NOT NULL, role TEXT NOT NULL, body TEXT NOT NULL,
                     timestamp TEXT NOT NULL, seq INTEGER NOT NULL DEFAULT 0);",
            )
            .unwrap();
            // A legacy row predating the new columns.
            conn.execute(
                "INSERT INTO sessions
                   (session_id, agent_id, source, last_seq, created_at, last_message_at)
                 VALUES ('h-old', '@a', 'claude', 1, 't', 't')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages
                   (id, agent_id, session_id, chat_kind, role, body, timestamp, seq)
                 VALUES ('m-old', '@a', 'h-old', 'session', 'agent', 'legacy body', 't', 1)",
                [],
            )
            .unwrap();
        }

        with_connection(tmp.path(), |conn| {
            // New columns now exist on both tables.
            for (table, column) in [
                ("sessions", "status_state"),
                ("sessions", "current_detail"),
                ("sessions", "active_call_id"),
                ("sessions", "title"),
                ("sessions", "model"),
                ("sessions", "handle"),
                ("sessions", "repo"),
                ("sessions", "branch"),
                ("sessions", "capabilities"),
                ("messages", "event_kind"),
                ("messages", "tool_name"),
                ("messages", "call_id"),
                ("messages", "ok"),
                ("messages", "is_error"),
                ("messages", "exit_code"),
            ] {
                assert!(
                    column_exists(conn, table, column)?,
                    "{table}.{column} must be added by migration"
                );
            }

            // The legacy rows still read; the new fields come back NULL (None).
            let old = load_session(conn, "@a", "h-old")?.expect("legacy session survives");
            assert_eq!(old.source, "claude");
            assert_eq!(old.status_state, None);
            assert_eq!(old.current_detail, None);
            let msgs = list_recent_messages(conn, "@a", "h-old", 10)?;
            assert_eq!(msgs.len(), 1);
            assert_eq!(msgs[0].body, "legacy body");
            assert_eq!(msgs[0].event_kind, None);
            assert_eq!(msgs[0].ok, None);
            assert_eq!(msgs[0].is_error, None);
            assert_eq!(msgs[0].exit_code, None);

            // And the upgraded schema accepts writes that populate the new fields.
            upsert_session(
                conn,
                &OrchestrationSession {
                    status_state: Some("running".into()),
                    current_detail: Some("compiling".into()),
                    active_call_id: Some("call-1".into()),
                    ..session("@a", "h-old", 1)
                },
            )?;
            let updated = load_session(conn, "@a", "h-old")?.unwrap();
            assert_eq!(updated.status_state.as_deref(), Some("running"));
            assert_eq!(updated.current_detail.as_deref(), Some("compiling"));
            assert_eq!(updated.active_call_id.as_deref(), Some("call-1"));
            let tool_result = OrchestrationMessage {
                event_kind: Some("tool_result".into()),
                tool_name: Some("Bash".into()),
                call_id: Some("call-1".into()),
                ok: Some(false),
                is_error: Some(true),
                exit_code: Some(1),
                ..msg("m-new", "@a", "h-old", 2)
            };
            assert!(insert_message(conn, &tool_result)?);
            let upgraded_messages = list_recent_messages(conn, "@a", "h-old", 10)?;
            let saved = upgraded_messages
                .iter()
                .find(|m| m.id == "m-new")
                .expect("upgraded schema stores outcome fields");
            assert_eq!(saved.ok, Some(false));
            assert_eq!(saved.is_error, Some(true));
            assert_eq!(saved.exit_code, Some(1));
            Ok(())
        })
        .unwrap();

        // Re-opening is idempotent — the ADD COLUMN guard must not error the second
        // time (no `duplicate column name`).
        with_connection(tmp.path(), |conn| {
            assert!(column_exists(conn, "messages", "call_id")?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn with_connection_enables_wal_journal_mode() {
        // WAL is what lets the one-time `migrate()` ALTERs run without being
        // starved into SQLITE_BUSY by a concurrent reader (see with_connection).
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
            assert_eq!(mode.to_lowercase(), "wal", "orchestration DB runs in WAL");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migrates_pre_v2_schema_while_a_reader_is_open() {
        // Reproduces the "migrate orchestration schema" failure: a legacy store
        // missing the v2 columns is opened while another connection holds a read
        // lock (the drain loop). Under WAL the schema-modifying ADD COLUMNs must
        // still succeed instead of timing out on the reader's lock.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("orchestration").join("orchestration.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (
                     session_id TEXT NOT NULL, agent_id TEXT NOT NULL, source TEXT NOT NULL,
                     label TEXT, workspace TEXT, last_seq INTEGER NOT NULL DEFAULT 0,
                     created_at TEXT NOT NULL, last_message_at TEXT NOT NULL,
                     PRIMARY KEY (agent_id, session_id));
                 CREATE TABLE messages (
                     id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, session_id TEXT NOT NULL,
                     chat_kind TEXT NOT NULL, role TEXT NOT NULL, body TEXT NOT NULL,
                     timestamp TEXT NOT NULL, seq INTEGER NOT NULL DEFAULT 0);",
            )
            .unwrap();
        }

        // A second connection sitting on an open read transaction, mimicking the
        // drain having the DB open when the UI's sessions_list triggers migrate.
        let reader = Connection::open(&db_path).unwrap();
        reader
            .busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        reader
            .query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
            .unwrap();
        reader
            .execute_batch("BEGIN; SELECT COUNT(*) FROM messages;")
            .unwrap();

        // Migration through with_connection must not be starved by the reader.
        with_connection(tmp.path(), |conn| {
            assert!(column_exists(conn, "messages", "ok")?);
            assert!(column_exists(conn, "sessions", "capabilities")?);
            Ok(())
        })
        .expect("migration succeeds despite a concurrently-held reader");

        reader.execute_batch("COMMIT").unwrap();
    }

    #[test]
    fn in_immediate_txn_serialises_concurrent_seq_allocation() {
        // Two writers on the same (agent, session) — the drain's inbound persist
        // and the graph's send_dm reply persist — must not read the same
        // MAX(seq) and duplicate it. Allocate + insert under `in_immediate_txn`
        // from several threads and assert every seq is distinct and contiguous.
        use std::sync::Arc;
        let tmp = Arc::new(tempfile::tempdir().unwrap());
        let n = 8usize;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let tmp = Arc::clone(&tmp);
                std::thread::spawn(move || {
                    with_connection(tmp.path(), |c| {
                        in_immediate_txn(c, |c| {
                            let seq = next_session_seq(c, "@peer", "s1")?;
                            insert_message(c, &msg(&format!("m{i}"), "@peer", "s1", seq))?;
                            Ok(seq)
                        })
                    })
                    .expect("txn ok")
                })
            })
            .collect();
        let mut seqs: Vec<i64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        seqs.sort_unstable();
        let mut unique = seqs.clone();
        unique.dedup();
        assert_eq!(
            unique.len(),
            n,
            "concurrent seq allocation must not duplicate: {seqs:?}"
        );
        assert_eq!(
            seqs,
            (1..=n as i64).collect::<Vec<_>>(),
            "seqs must be a contiguous 1..=n range"
        );
    }
}
