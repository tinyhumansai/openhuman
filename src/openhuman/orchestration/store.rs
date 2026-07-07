//! SQLite persistence for the orchestration domain.
//!
//! Lives at `<workspace>/orchestration/orchestration.db`. Message bodies are
//! decrypted plaintext, so this path is workspace-internal (protected by
//! `is_workspace_internal_path`). Follows the subconscious/cron `with_connection`
//! pattern.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::types::{OrchestrationMessage, OrchestrationSession};

const SCHEMA_DDL: &str = "
    PRAGMA foreign_keys = ON;

    -- `status_state`/`current_detail`/`active_call_id` carry the v2 harness
    -- run-state (`status.state`/`status.detail`/`status.active_call_id`). Nullable
    -- and additive: a v1/legacy store gets them via `migrate` (existing rows NULL).
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
        PRIMARY KEY (agent_id, session_id)
    );

    -- `event_kind`/`tool_name`/`call_id` carry the v2 per-message event shape
    -- (`event.kind` + tool identity/correlation). Nullable and additive; v1 and
    -- pinned master/subconscious rows leave them NULL.
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
        call_id    TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_messages_session
        ON messages (agent_id, session_id, timestamp);

    CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);

    -- Stage 5: 20:1-compressed execution-trace summaries, one row per wake cycle.
    -- Keyed by cycle_id so a checkpoint-resumed cycle re-writes idempotently.
    CREATE TABLE IF NOT EXISTS compressed_history (
        cycle_id      TEXT PRIMARY KEY,
        session_id    TEXT NOT NULL,
        agent_id      TEXT NOT NULL,
        input_tokens  INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        text          TEXT NOT NULL,
        created_at    TEXT NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_compressed_session
        ON compressed_history (agent_id, session_id, created_at);

    -- Stage 5: append-only world-state-diff timeline. `seq` is monotonic per
    -- (agent, session) from genesis (seq 1). Keyed by cycle_id so a resumed
    -- cycle never appends a duplicate row.
    CREATE TABLE IF NOT EXISTS world_diff (
        cycle_id        TEXT PRIMARY KEY,
        seq             INTEGER NOT NULL,
        session_id      TEXT NOT NULL,
        agent_id        TEXT NOT NULL,
        event_signature TEXT NOT NULL,
        world_mutation  TEXT NOT NULL,
        delta           TEXT NOT NULL,
        timestamp       TEXT NOT NULL
    );

    -- The UNIQUE index on (agent_id, session_id, seq) — which makes a racing
    -- `MAX(seq)+1` allocation impossible to persist twice — is created by the
    -- one-time `migrate()` step (user_version-gated), not here: the initial
    -- release shipped a NON-unique `idx_world_diff_session`, and
    -- `CREATE UNIQUE INDEX IF NOT EXISTS` under that name is a no-op on stores
    -- that already have it. See `migrate`.

    -- Stage 6: append-only subconscious steering directives. 'Current' directive
    -- is the latest row with superseded_by IS NULL that has not expired
    -- (created_cycle + expires_after_cycles > current cycle). Never rewritten.
    CREATE TABLE IF NOT EXISTS steering_directives (
        id                  INTEGER PRIMARY KEY AUTOINCREMENT,
        text                TEXT NOT NULL,
        created_at          TEXT NOT NULL,
        source_tick_id      TEXT NOT NULL,
        expires_after_cycles INTEGER NOT NULL,
        created_cycle       INTEGER NOT NULL,
        derived_from        TEXT NOT NULL,
        superseded_by       INTEGER
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
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    // v1 — enforce uniqueness of the world-diff timeline position.
    // The initial release created `idx_world_diff_session` NON-unique, so a
    // race between concurrent `MAX(seq)+1` allocations could persist duplicate
    // `(agent_id, session_id, seq)` rows. Drop the legacy index, de-dupe any
    // pre-existing race rows (keep the earliest per key), create the unique
    // index under a new name, then reconcile `terminal_state` so it can't point
    // at a mutation from a deleted duplicate. Runs once (guarded), so it never
    // rewrites `terminal_state` on steady-state opens.
    if version < 1 {
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_world_diff_session;
             DELETE FROM world_diff WHERE rowid NOT IN (
                 SELECT MIN(rowid) FROM world_diff GROUP BY agent_id, session_id, seq
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_world_diff_session_uniq
                 ON world_diff (agent_id, session_id, seq);
             INSERT OR REPLACE INTO kv (k, v)
                 SELECT 'terminal_state:' || wd.agent_id || ':' || wd.session_id,
                        wd.world_mutation
                 FROM world_diff wd
                 WHERE wd.seq = (
                     SELECT MAX(seq) FROM world_diff w2
                     WHERE w2.agent_id = wd.agent_id AND w2.session_id = wd.session_id
                 );
             PRAGMA user_version = 1;",
        )?;
    }

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

/// Insert or update the session row (keyed by agent + session).
pub fn upsert_session(conn: &Connection, s: &OrchestrationSession) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions
           (session_id, agent_id, source, label, workspace, last_seq, created_at, last_message_at,
            status_state, current_detail, active_call_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(agent_id, session_id) DO UPDATE SET
           last_seq = MAX(sessions.last_seq, excluded.last_seq),
           last_message_at = excluded.last_message_at,
           label = COALESCE(excluded.label, sessions.label),
           workspace = COALESCE(excluded.workspace, sessions.workspace),
           -- Run-state fields COALESCE so a content event (which carries none)
           -- never wipes the last status; a fresh `status` event overwrites them.
           status_state = COALESCE(excluded.status_state, sessions.status_state),
           current_detail = COALESCE(excluded.current_detail, sessions.current_detail),
           active_call_id = COALESCE(excluded.active_call_id, sessions.active_call_id)",
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
        ],
    )?;
    Ok(())
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
            event_kind, tool_name, call_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                      OR event_kind NOT IN ('status', 'lifecycle', 'unknown'))
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
                status_state, current_detail, active_call_id
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
                        event_kind, tool_name, call_id
                   FROM messages WHERE session_id = ?1 AND timestamp < ?2
                     AND (event_kind IS NULL
                          OR event_kind NOT IN ('status', 'lifecycle', 'unknown'))
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
                        event_kind, tool_name, call_id
                   FROM messages WHERE session_id = ?1
                     AND (event_kind IS NULL
                          OR event_kind NOT IN ('status', 'lifecycle', 'unknown'))
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
    })
}

/// Count unread messages for a chat: rows with `timestamp` after the read cursor.
pub fn unread_count(conn: &Connection, session_id: &str) -> Result<i64> {
    let cursor = kv_get(conn, &read_cursor_key(session_id))?.unwrap_or_default();
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND timestamp > ?2
             AND (event_kind IS NULL
                  OR event_kind NOT IN ('status', 'lifecycle', 'unknown'))",
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

/// Load a single session row (the wake graph's counterpart + metadata).
pub fn load_session(
    conn: &Connection,
    agent_id: &str,
    session_id: &str,
) -> Result<Option<OrchestrationSession>> {
    conn.query_row(
        "SELECT session_id, agent_id, source, label, workspace, last_seq, created_at, last_message_at,
                status_state, current_detail, active_call_id
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
                event_kind, tool_name, call_id
           FROM messages WHERE agent_id = ?1 AND session_id = ?2
           ORDER BY timestamp DESC, seq DESC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![agent_id, session_id, limit], map_message_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Reverse the DESC scan back to chronological order.
    Ok(rows.into_iter().rev().collect())
}

/// Insert a compressed-history row, idempotent by `cycle_id`. Returns true if a
/// new row landed (false on a resumed-cycle replay).
#[allow(clippy::too_many_arguments)]
pub fn insert_compressed(
    conn: &Connection,
    cycle_id: &str,
    session_id: &str,
    agent_id: &str,
    input_tokens: i64,
    output_tokens: i64,
    text: &str,
    created_at: &str,
) -> Result<bool> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO compressed_history
           (cycle_id, session_id, agent_id, input_tokens, output_tokens, text, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            cycle_id,
            session_id,
            agent_id,
            input_tokens,
            output_tokens,
            text,
            created_at
        ],
    )?;
    Ok(changed > 0)
}

/// Count compressed-history rows for a session.
pub fn count_compressed(conn: &Connection, agent_id: &str, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM compressed_history WHERE agent_id = ?1 AND session_id = ?2",
        params![agent_id, session_id],
        |row| row.get(0),
    )?)
}

/// Append one world-diff timeline entry, idempotent by `cycle_id`. The `seq` is
/// assigned monotonically per (agent, session) — genesis is seq 1. Returns the
/// assigned seq for a new row, or the existing row's seq on a resumed replay
/// (never a second row). Also stamps `terminal_state:<agent>:<session>` in `kv`.
#[allow(clippy::too_many_arguments)]
pub fn append_world_diff(
    conn: &Connection,
    cycle_id: &str,
    session_id: &str,
    agent_id: &str,
    event_signature: &str,
    world_mutation: &str,
    delta: &str,
    timestamp: &str,
) -> Result<i64> {
    // Idempotent replay: if this cycle already appended, return its seq unchanged.
    if let Some(seq) = conn
        .query_row(
            "SELECT seq FROM world_diff WHERE cycle_id = ?1",
            params![cycle_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(seq);
    }

    let next_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM world_diff WHERE agent_id = ?1 AND session_id = ?2",
        params![agent_id, session_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO world_diff
           (cycle_id, seq, session_id, agent_id, event_signature, world_mutation, delta, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            cycle_id,
            next_seq,
            session_id,
            agent_id,
            event_signature,
            world_mutation,
            delta,
            timestamp
        ],
    )?;
    kv_set(
        conn,
        &format!("terminal_state:{agent_id}:{session_id}"),
        world_mutation,
    )?;
    Ok(next_seq)
}

/// The ordered `seq` values of a session's world-diff timeline (append-only test
/// + stage-7 read surface).
pub fn world_diff_seqs(conn: &Connection, agent_id: &str, session_id: &str) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT seq FROM world_diff WHERE agent_id = ?1 AND session_id = ?2 ORDER BY seq ASC",
    )?;
    let rows = stmt
        .query_map(params![agent_id, session_id], |r| r.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ── Stage 6: steering directives + review cursor ────────────────────────────

use super::steering::SteeringDirective;

/// Kv key: the global reasoning-cycle counter (bumped once per wake cycle).
const CYCLE_COUNTER_KEY: &str = "orchestration:cycle";
/// Kv key: the `created_at` high-water mark of reviewed compressed-history rows.
const REVIEW_CURSOR_KEY: &str = "steering:reviewed_at";

/// Bump and return the global reasoning-cycle counter. Called once per wake cycle
/// so steering-directive expiry can be measured in cycles.
pub fn bump_cycle_counter(conn: &Connection) -> Result<i64> {
    let current: i64 = kv_get(conn, CYCLE_COUNTER_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let next = current + 1;
    kv_set(conn, CYCLE_COUNTER_KEY, &next.to_string())?;
    Ok(next)
}

/// The current reasoning-cycle counter (read-only; used at directive creation).
pub fn current_cycle_counter(conn: &Connection) -> Result<i64> {
    Ok(kv_get(conn, CYCLE_COUNTER_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

/// The review cursor: the highest compressed-history `created_at` already folded
/// into a steering tick (empty string until the first review).
pub fn review_cursor(conn: &Connection) -> Result<String> {
    Ok(kv_get(conn, REVIEW_CURSOR_KEY)?.unwrap_or_default())
}

/// Advance the review cursor (idempotent — only after a successful persist).
pub fn set_review_cursor(conn: &Connection, created_at: &str) -> Result<()> {
    kv_set(conn, REVIEW_CURSOR_KEY, created_at)
}

/// Compressed-history rows not yet reviewed (created_at > cursor), oldest-first,
/// bounded. Returns `(created_at, text)`.
pub fn list_unreviewed_compressed(
    conn: &Connection,
    since_created_at: &str,
    limit: u32,
) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT created_at, text FROM compressed_history
           WHERE created_at > ?1 ORDER BY created_at ASC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![since_created_at, limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The most recent world-diff mutations across all sessions (the cumulative
/// timeline), oldest-first within the returned window.
pub fn list_recent_world_mutations(conn: &Connection, limit: u32) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT world_mutation FROM world_diff ORDER BY timestamp DESC, seq DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().rev().collect())
}

/// Append a steering directive, superseding the prior current directive. Returns
/// the new directive's id.
pub fn insert_steering_directive(
    conn: &Connection,
    text: &str,
    created_at: &str,
    source_tick_id: &str,
    expires_after_cycles: u32,
    created_cycle: i64,
    derived_from: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO steering_directives
           (text, created_at, source_tick_id, expires_after_cycles, created_cycle, derived_from)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            text,
            created_at,
            source_tick_id,
            expires_after_cycles,
            created_cycle,
            derived_from
        ],
    )?;
    let new_id = conn.last_insert_rowid();
    // Supersede every prior still-current directive so 'current' is unambiguous.
    conn.execute(
        "UPDATE steering_directives SET superseded_by = ?1
           WHERE superseded_by IS NULL AND id <> ?1",
        params![new_id],
    )?;
    Ok(new_id)
}

/// The current directive: the latest non-superseded row that has not expired at
/// `current_cycle` (`created_cycle + expires_after_cycles > current_cycle`).
pub fn current_steering_directive(
    conn: &Connection,
    current_cycle: i64,
) -> Result<Option<SteeringDirective>> {
    conn.query_row(
        "SELECT id, text, created_at, expires_after_cycles, created_cycle
           FROM steering_directives
           WHERE superseded_by IS NULL
             AND (created_cycle + expires_after_cycles) > ?1
           ORDER BY id DESC LIMIT 1",
        params![current_cycle],
        |row| {
            Ok(SteeringDirective {
                id: row.get(0)?,
                text: row.get(1)?,
                created_at: row.get(2)?,
                expires_after_cycles: row.get::<_, i64>(3)? as u32,
                created_cycle: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
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
    fn status_lifecycle_unknown_rows_are_hidden_from_thread_and_unread() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            upsert_session(conn, &session("@a", "h1", 1))?;

            // A v1 row (no event_kind) and typed content rows stay visible;
            // status/lifecycle/unknown are persisted (for relay dedup) but must
            // not surface in the thread or the unread count.
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
            ] {
                let mut hidden = msg(id, "@a", "h1", seq);
                hidden.event_kind = Some(kind.into());
                hidden.timestamp = format!("2026-07-02T00:00:0{seq}Z");
                insert_message(conn, &hidden)?;
            }

            let thread = list_messages_by_session(conn, "h1", 50, None)?;
            let ids: Vec<&str> = thread.iter().map(|m| m.id.as_str()).collect();
            assert_eq!(ids, vec!["v1", "call"], "only v1 + typed content rows");

            // Unread (cursor at 0) counts the two visible rows, not the 3 hidden.
            assert_eq!(unread_count(conn, "h1")?, 2);

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
    fn world_diff_is_append_only_with_monotonic_seq_and_idempotent_cycles() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            // Genesis is seq 1, next cycle seq 2 — the append-only timeline.
            let s1 = append_world_diff(conn, "h1#1", "h1", "@a", "sig1", "world v1", "d1", "t1")?;
            let s2 = append_world_diff(conn, "h1#2", "h1", "@a", "sig2", "world v2", "d2", "t2")?;
            assert_eq!(s1, 1, "genesis seq");
            assert_eq!(s2, 2, "second cycle seq");

            // A resumed cycle (same cycle_id) does not append a second row and
            // returns the original seq — genesis untouched.
            let s1_again =
                append_world_diff(conn, "h1#1", "h1", "@a", "sig1", "world v1'", "d1'", "t1'")?;
            assert_eq!(s1_again, 1, "resumed cycle reuses its seq");
            assert_eq!(
                world_diff_seqs(conn, "@a", "h1")?,
                vec![1, 2],
                "no duplicate rows"
            );

            // terminal_state tracks the latest mutation.
            assert_eq!(
                kv_get(conn, "terminal_state:@a:h1")?.as_deref(),
                Some("world v2")
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn steering_supersede_chain_and_expiry_by_cycle_count() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            // Directive A created at cycle 5, expires after 10 → valid until 15.
            let a = insert_steering_directive(conn, "A", "t1", "tick1", 10, 5, "rows:1-2")?;
            let cur = current_steering_directive(conn, 6)?.expect("current at cycle 6");
            assert_eq!(cur.id, a);
            assert_eq!(cur.text, "A");

            // Directive B supersedes A. Now B is current, A is superseded.
            let b = insert_steering_directive(conn, "B", "t2", "tick2", 10, 8, "rows:3")?;
            let cur = current_steering_directive(conn, 9)?.expect("current at cycle 9");
            assert_eq!(cur.id, b, "newest non-superseded directive wins");

            // B (created cycle 8, expires 10) is expired once cycle ≥ 18.
            assert!(
                current_steering_directive(conn, 17)?.is_some(),
                "still valid at 17"
            );
            assert!(
                current_steering_directive(conn, 18)?.is_none(),
                "expired at 18"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn cycle_counter_bumps_and_review_cursor_advances() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            assert_eq!(current_cycle_counter(conn)?, 0);
            assert_eq!(bump_cycle_counter(conn)?, 1);
            assert_eq!(bump_cycle_counter(conn)?, 2);
            assert_eq!(current_cycle_counter(conn)?, 2);

            assert_eq!(review_cursor(conn)?, "");
            insert_compressed(
                conn,
                "h1#1",
                "h1",
                "@a",
                100,
                5,
                "s1",
                "2026-07-02T00:00:01Z",
            )?;
            insert_compressed(
                conn,
                "h1#2",
                "h1",
                "@a",
                100,
                5,
                "s2",
                "2026-07-02T00:00:02Z",
            )?;
            let unreviewed = list_unreviewed_compressed(conn, "", 10)?;
            assert_eq!(unreviewed.len(), 2);
            set_review_cursor(conn, "2026-07-02T00:00:01Z")?;
            let after = list_unreviewed_compressed(conn, &review_cursor(conn)?, 10)?;
            assert_eq!(after.len(), 1, "only the newer row remains unreviewed");
            assert_eq!(after[0].1, "s2");
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
    fn compressed_history_is_idempotent_by_cycle_id() {
        let tmp = tempfile::tempdir().unwrap();
        with_connection(tmp.path(), |conn| {
            assert!(insert_compressed(
                conn, "h1#1", "h1", "@a", 400, 20, "summary", "now"
            )?);
            // Resumed cycle → no second row.
            assert!(!insert_compressed(
                conn, "h1#1", "h1", "@a", 400, 20, "summary", "now"
            )?);
            assert_eq!(count_compressed(conn, "@a", "h1")?, 1);
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
    fn migrates_legacy_nonunique_world_diff_index_to_unique() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("orchestration").join("orchestration.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Simulate a pre-migration store: the world_diff table with the OLD
        // NON-unique index, holding two rows that share (agent, session, seq) —
        // the exact duplicate the concurrent `MAX(seq)+1` race could produce.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE world_diff (
                    cycle_id TEXT PRIMARY KEY, seq INTEGER NOT NULL, session_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL, event_signature TEXT NOT NULL,
                    world_mutation TEXT NOT NULL, delta TEXT NOT NULL, timestamp TEXT NOT NULL);
                 CREATE INDEX idx_world_diff_session ON world_diff (agent_id, session_id, seq);",
            )
            .unwrap();
            // Distinct mutations so we can prove terminal_state is reconciled.
            conn.execute(
                "INSERT INTO world_diff VALUES ('c1', 1, 's', '@a', 'sig', 'mut_c1', 'd', 't')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO world_diff VALUES ('c2', 1, 's', '@a', 'sig', 'mut_c2', 'd', 't')",
                [],
            )
            .unwrap();
            conn.execute("CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)", [])
                .unwrap();
            // Stale terminal_state pointing at the duplicate that will be deleted.
            conn.execute(
                "INSERT INTO kv VALUES ('terminal_state:@a:s', 'mut_c2')",
                [],
            )
            .unwrap();
        }

        // Opening through with_connection applies SCHEMA_DDL + the migration.
        with_connection(tmp.path(), |conn| {
            // Legacy duplicate race rows are de-duped to one per (agent, session, seq).
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM world_diff WHERE agent_id='@a' AND session_id='s' AND seq=1",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(n, 1, "duplicate legacy rows de-duped");

            // terminal_state is reconciled to the surviving row's mutation (not the deleted one).
            let ts: String =
                conn.query_row("SELECT v FROM kv WHERE k='terminal_state:@a:s'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(
                ts, "mut_c1",
                "terminal_state reconciled to the surviving row"
            );

            // The index is now UNIQUE — a fresh duplicate (agent, session, seq) is rejected.
            let dup = conn.execute(
                "INSERT INTO world_diff VALUES ('c3', 1, 's', '@a', 'sig', 'mut', 'd', 't')",
                [],
            );
            assert!(dup.is_err(), "unique index rejects a duplicate seq");

            // Migration is one-shot: user_version was bumped.
            let uv: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            assert_eq!(uv, 1, "migration marks user_version");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migrates_pre_v2_schema_by_adding_status_and_event_columns() {
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
                ("messages", "event_kind"),
                ("messages", "tool_name"),
                ("messages", "call_id"),
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
