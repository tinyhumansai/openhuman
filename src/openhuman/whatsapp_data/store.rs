//! SQLite-backed persistence for structured WhatsApp Web data.
//!
//! Data is stored in a dedicated `whatsapp_data.db` file inside the
//! workspace directory. Tables: `wa_chats` and `wa_messages`.
//!
//! This store is local-only; no data is transmitted to external services.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::openhuman::whatsapp_data::sqlite_retry::{
    is_sqlite_corrupt, retry_on_sqlite_busy, BUSY_TIMEOUT,
};
use crate::openhuman::whatsapp_data::types::{
    ChatMeta, IngestMessage, ListChatsRequest, ListMessagesRequest, SearchMessagesRequest,
    WhatsAppChat, WhatsAppMessage,
};

/// Process-wide latch so a `SQLITE_CORRUPT` flood is reported to Sentry
/// **once**, not on every scan tick. The whatsapp_scanner re-ingests every
/// 2–30s, so a wedged DB re-hits the malformed-image error on each poll — one
/// corrupt file produced 1,813 escalating Sentry events from a single host
/// (TAURI-RUST-KNH). Set on the first detection; cleared once a recovery
/// attempt settles (quarantine + rebuild, or a quick_check that now passes) so
/// a genuinely-new, later corruption can still page exactly once.
static CORRUPT_REPORTED: AtomicBool = AtomicBool::new(false);

/// Test-only override: when set, [`WhatsAppDataStore::integrity_check_ok`]
/// reports the (rebuilt) DB as failing its integrity check. This is the seam
/// that lets tests drive the "rebuild still fails integrity_check" branch of
/// [`WhatsAppDataStore::recover_corrupt_db`] without forging a file that both
/// survives quarantine and yet fails `PRAGMA integrity_check`.
#[cfg(test)]
static FORCE_INTEGRITY_CHECK_FAIL: AtomicBool = AtomicBool::new(false);

/// Test-only serialization guard for the recovery-episode tests. `CORRUPT_REPORTED`
/// and `FORCE_INTEGRITY_CHECK_FAIL` are process-wide, so tests that mutate them
/// must not run concurrently or they clobber each other's latch observations.
#[cfg(test)]
static CORRUPT_TEST_GUARD: Mutex<()> = Mutex::new(());

/// SQLite-backed store for WhatsApp chats and messages.
pub struct WhatsAppDataStore {
    db_path: std::path::PathBuf,
    /// Serializes write paths (upsert + prune) so concurrent ingest RPCs do not
    /// open competing writers on the same `whatsapp_data.db` file.
    write_lock: Mutex<()>,
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
        let store = Self {
            db_path,
            write_lock: Mutex::new(()),
        };
        store.init_schema_or_recover()?;
        Ok(store)
    }

    /// Initialize the schema, self-healing a DB that is already corrupt **at
    /// process startup**.
    ///
    /// Without this, a `whatsapp_data.db` that is malformed before the first
    /// write RPC arrives makes `init_schema()` fail, `global::init` leaves the
    /// store singleton unset, and every subsequent ingest RPC fails with
    /// "store accessed before init" — the corruption never reaches the
    /// quarantine + rebuild path (which only guards the *write* wrapper), so it
    /// survives restarts and re-pages Sentry on every scanner tick. Recovering
    /// here makes a boot-time corrupt DB heal exactly as a mid-run one does.
    ///
    /// The `CORRUPT_REPORTED` latch semantics are reused verbatim via
    /// [`Self::report_and_recover`] (report once per episode, reset only on a
    /// confirmed-healthy rebuild), so a boot-time corruption pages at most once.
    fn init_schema_or_recover(&self) -> Result<()> {
        match self.init_schema() {
            Ok(()) => Ok(()),
            Err(e) if is_sqlite_corrupt(&e) => {
                log::error!(
                    "[whatsapp_data] init_schema hit SQLITE_CORRUPT at startup for {} — \
                     driving quarantine + rebuild before ingest can begin: {e:#}",
                    self.db_path.display()
                );
                // Reports once (latch), quarantines the malformed image, and
                // rebuilds the schema. `recover_corrupt_db` is safe to call on
                // the just-constructed store: it only needs `db_path` +
                // `init_schema`, both available before the store is published.
                self.report_and_recover("init_schema", &e);
                // Confirm the store is now usable. If recovery failed, this
                // re-init returns Err and `global::init` correctly leaves the
                // singleton unset — but now only when the DB is genuinely
                // unrecoverable, not merely corrupt-on-boot.
                self.init_schema()
                    .context("re-init whatsapp_data schema after boot-time corrupt-DB recovery")
            }
            Err(e) => Err(e),
        }
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

    /// Open a fresh connection to the DB file.
    ///
    /// Read paths (`list_chats` / `list_messages` / `search_messages`) call this
    /// **without** taking `write_lock`, so a read can race the brief file-rename
    /// window inside [`Self::recover_corrupt_db`] (which quarantines the corrupt
    /// image then rebuilds under `write_lock`). We deliberately do NOT serialize
    /// reads behind the write lock: the race is not a correctness or
    /// data-integrity problem, only a rare transient read error that self-heals
    /// on the next poll. Three outcomes are possible and all are safe —
    /// (1) the read opens before the rename and reads valid (old) data via its
    /// still-live fd; (2) it opens after the rebuild and reads the fresh schema;
    /// (3) it opens in the sub-millisecond gap after the rename but before the
    /// rebuild, gets an empty auto-created file, and returns a benign
    /// "no such table" error the caller retries. Guarding this hot path with a
    /// global read/write lock would trade that vanishingly-rare transient for
    /// permanent read/write contention on every list/search — not worth it.
    fn open_conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("open whatsapp_data db: {}", self.db_path.display()))?;
        Self::configure_connection(&conn)?;
        Ok(conn)
    }

    /// Per-connection pragmas: busy handler + WAL (idempotent on existing DBs).
    fn configure_connection(conn: &Connection) -> Result<()> {
        conn.busy_timeout(BUSY_TIMEOUT)
            .context("configure whatsapp_data busy_timeout")?;
        if let Err(wal_err) = conn.execute_batch("PRAGMA journal_mode=WAL;") {
            log::warn!(
                "[whatsapp_data] failed to enable WAL (filesystem may not support it): {wal_err}"
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn open_conn_for_test(&self) -> Result<Connection> {
        self.open_conn()
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Run a write op through the busy-retry loop, and on a confirmed
    /// `SQLITE_CORRUPT` malformed-image error drive a **once-per-call**
    /// quarantine + rebuild recovery, then retry the op a single time against
    /// the rebuilt schema.
    ///
    /// The recovery is bounded: at most one quarantine attempt and one retry
    /// per public write call. A genuinely unrecoverable file therefore returns
    /// the corrupt error to the caller after one recovery pass instead of
    /// spinning — the process-wide report latch (`report_and_recover`) then
    /// keeps Sentry from re-flooding on every scan tick.
    fn write_with_corrupt_recovery<T>(
        &self,
        op_name: &str,
        f: impl Fn() -> Result<T>,
    ) -> Result<T> {
        match retry_on_sqlite_busy(op_name, &f) {
            Ok(val) => Ok(val),
            Err(e) if is_sqlite_corrupt(&e) => {
                self.report_and_recover(op_name, &e);
                // Retry once against the (now rebuilt) DB. If this still fails,
                // the error propagates — no further recovery, so a wedged
                // filesystem can't loop.
                retry_on_sqlite_busy(op_name, &f)
            }
            Err(e) => Err(e),
        }
    }

    /// Report a confirmed `SQLITE_CORRUPT` failure to Sentry and drive the
    /// quarantine + rebuild recovery. Factored out of
    /// [`Self::write_with_corrupt_recovery`] so the report + recovery decision
    /// is unit-testable without a live scanner loop.
    fn report_and_recover(&self, op_name: &str, err: &anyhow::Error) {
        // Report to Sentry at most once per corruption episode. Without this
        // latch the scanner's 2–30s poll re-hits the wedged DB and re-pages on
        // every tick (TAURI-RUST-KNH: 1,813 events from one host).
        if !CORRUPT_REPORTED.swap(true, Ordering::Relaxed) {
            crate::core::observability::report_error(
                err,
                "whatsapp_data",
                "ingest_corrupt",
                &[("op", op_name)],
            );
        }
        log::error!(
            "[whatsapp_data] {op_name} hit SQLITE_CORRUPT (malformed DB image), \
             attempting quarantine + rebuild recovery: {err:#}"
        );
        match self.recover_corrupt_db() {
            Ok(true) => {
                log::warn!(
                    "[whatsapp_data] {op_name} quarantined corrupt DB and rebuilt empty schema; \
                     ingest will resume"
                );
                // Recovery settled — allow a future, genuinely-new corruption
                // to page once more.
                CORRUPT_REPORTED.store(false, Ordering::Relaxed);
            }
            Ok(false) => {
                log::info!(
                    "[whatsapp_data] {op_name} corruption recovery: integrity check now passes, \
                     no quarantine needed"
                );
                CORRUPT_REPORTED.store(false, Ordering::Relaxed);
            }
            Err(rec_err) => log::error!(
                "[whatsapp_data] {op_name} corruption recovery FAILED, ingest stays degraded: \
                 {rec_err:#}"
            ),
        }
    }

    /// Recover from a `SQLITE_CORRUPT` (malformed image) on the whatsapp_data DB.
    ///
    /// A malformed on-disk image never heals on its own — every write fails
    /// forever and the scanner re-pages Sentry on each poll (Sentry
    /// TAURI-RUST-KNH: 1,813 events from a single host). This quarantines the
    /// damaged file (and its WAL/SHM side-files) to a timestamped
    /// `.corrupt-<ts>` copy — **preserved, not deleted**, so the bytes can be
    /// inspected or salvaged — then rebuilds an empty schema so ingest resumes.
    ///
    /// Returns `Ok(true)` when a quarantine + rebuild happened, `Ok(false)`
    /// when a fresh `PRAGMA quick_check` now passes (the earlier failure was
    /// transient and quarantining would have destroyed good data), and `Err`
    /// when the quarantine rename or the schema rebuild failed.
    ///
    /// The store opens a fresh `Connection` per call (no cached handle), so no
    /// connection needs to be dropped before the rename — the next `open_conn`
    /// naturally picks up the rebuilt file.
    pub(crate) fn recover_corrupt_db(&self) -> Result<bool> {
        // 1. Re-confirm corruption against the on-disk file. `quick_check` is
        //    the cheap structural scan; if it now reports "ok" the image is
        //    actually healthy (e.g. the original error was a transient mmap
        //    fault) and we must NOT destroy good data — bail without quarantine.
        if self.db_path.exists() {
            match self.quick_check_ok() {
                Ok(true) => {
                    log::info!(
                        "[whatsapp_data] quick_check passed for {} — no quarantine needed",
                        self.db_path.display()
                    );
                    return Ok(false);
                }
                Ok(false) => {
                    log::warn!(
                        "[whatsapp_data] quick_check confirms corruption for {}, quarantining",
                        self.db_path.display()
                    );
                }
                Err(e) => {
                    // The check couldn't even run (unopenable / unreadable
                    // header). That is itself a malformed-image signal.
                    log::warn!(
                        "[whatsapp_data] quick_check could not run for {} ({e:#}); \
                         treating as corrupt",
                        self.db_path.display()
                    );
                }
            }
        } else {
            log::warn!(
                "[whatsapp_data] corrupt-recovery: {} is missing; rebuilding fresh schema",
                self.db_path.display()
            );
        }

        // 2. Quarantine the main DB + WAL/SHM side-files to `<name>.corrupt-<ts>`.
        let ts = Self::now_secs();
        let mut quarantined = 0usize;
        for suffix in &["", "-wal", "-shm"] {
            let src = with_name_suffix(&self.db_path, suffix);
            if !src.exists() {
                continue;
            }
            let dst = with_name_suffix(&src, &format!(".corrupt-{ts}"));
            std::fs::rename(&src, &dst).with_context(|| {
                format!(
                    "quarantine corrupt whatsapp_data file {} -> {}",
                    src.display(),
                    dst.display()
                )
            })?;
            log::warn!(
                "[whatsapp_data] quarantined {} -> {}",
                src.display(),
                dst.display()
            );
            quarantined += 1;
        }

        // 3. Rebuild an empty schema by re-running init on a fresh file. The
        //    damaged rows are not silently dropped — they live on in the
        //    `.corrupt-<ts>` copy.
        self.init_schema()
            .context("rebuild whatsapp_data schema after quarantining corrupt DB")?;

        // 4. Confirm the rebuilt image is structurally sound. This result is
        //    load-bearing: `report_and_recover` only resets the process-wide
        //    `CORRUPT_REPORTED` latch (and logs "ingest will resume") on
        //    `Ok(true)`. If the rebuild still fails integrity_check — or the
        //    check itself can't run — we MUST surface `Err`, otherwise the
        //    latch resets, re-arming Sentry to page on the next scan tick and
        //    breaking the report-once-per-episode guarantee this recovery
        //    exists to protect.
        match self.integrity_check_ok() {
            Ok(true) => {
                log::warn!(
                    "[whatsapp_data] corruption recovery complete: quarantined {quarantined} file(s), \
                     rebuilt empty schema, integrity_check=ok at {}",
                    self.db_path.display()
                );
                Ok(true)
            }
            Ok(false) => {
                log::error!(
                    "[whatsapp_data] rebuilt DB still fails integrity_check at {}",
                    self.db_path.display()
                );
                Err(anyhow::anyhow!(
                    "rebuilt whatsapp_data db still fails integrity_check at {}",
                    self.db_path.display()
                ))
            }
            Err(e) => Err(e.context("integrity_check after rebuild could not run")),
        }
    }

    /// Run `PRAGMA quick_check(1)` on a fresh, short-lived connection. Returns
    /// `Ok(true)` when the structural scan reports `"ok"`, `Ok(false)` on any
    /// reported corruption, and `Err` when the check itself can't run (file
    /// unopenable / header unreadable — itself a corruption signal the caller
    /// treats as malformed).
    fn quick_check_ok(&self) -> Result<bool> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("open for quick_check: {}", self.db_path.display()))?;
        let _ = conn.busy_timeout(BUSY_TIMEOUT);
        let result: String = conn
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .context("running PRAGMA quick_check")?;
        Ok(result.eq_ignore_ascii_case("ok"))
    }

    /// Run `PRAGMA integrity_check(1)` against the (rebuilt) DB to confirm it is
    /// structurally sound. Returns `Ok(true)` when it reports `"ok"`.
    fn integrity_check_ok(&self) -> Result<bool> {
        #[cfg(test)]
        if FORCE_INTEGRITY_CHECK_FAIL.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let conn = self.open_conn()?;
        let result: String = conn
            .query_row("PRAGMA integrity_check(1)", [], |row| row.get(0))
            .context("running PRAGMA integrity_check")?;
        Ok(result.eq_ignore_ascii_case("ok"))
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
        let _write_guard = self
            .write_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("whatsapp_data write lock poisoned: {e}"))?;
        self.write_with_corrupt_recovery("upsert_chats", || {
            self.upsert_chats_inner(account_id, chats)
        })
    }

    fn upsert_chats_inner(
        &self,
        account_id: &str,
        chats: &HashMap<String, ChatMeta>,
    ) -> Result<usize> {
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
        let _write_guard = self
            .write_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("whatsapp_data write lock poisoned: {e}"))?;
        self.write_with_corrupt_recovery("upsert_messages", || {
            self.upsert_messages_inner(account_id, msgs)
        })
    }

    fn upsert_messages_inner(&self, account_id: &str, msgs: &[IngestMessage]) -> Result<usize> {
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
        let _write_guard = self
            .write_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("whatsapp_data write lock poisoned: {e}"))?;
        self.write_with_corrupt_recovery("prune_old_messages", || {
            self.prune_old_messages_inner(cutoff_ts)
        })
    }

    fn prune_old_messages_inner(&self, cutoff_ts: i64) -> Result<u64> {
        let conn = self.open_conn()?;
        let now = Self::now_secs();

        // Collect affected (account_id, chat_id) pairs before deleting.
        // Contextualized so a `SQLITE_CORRUPT` surfaced while compiling this
        // scan still carries a `prune` frame marker the observability
        // classifier keys on (otherwise a boot-time prune corruption would
        // reach Sentry unfiltered — the prepare of a plain SELECT still reads
        // the schema/root page where damage commonly lives).
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT account_id, chat_id FROM wa_messages
                 WHERE timestamp > 0 AND timestamp < ?1",
            )
            .context("prune old wa_messages: scan affected chats")?;
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

        // Match against both `body` and `sender` so person-name queries like
        // "what did Alice say" surface Alice's messages even when "Alice"
        // does not appear in any message body. Branches are kept explicit so
        // the bind indices stay readable; each `pattern` bind is duplicated
        // because rusqlite does not resolve same-named placeholders for us.
        let msgs: Vec<WhatsAppMessage> = match (&req.account_id, &req.chat_id) {
            (Some(acct), Some(chat_id)) => {
                let mut stmt = conn.prepare(
                    "SELECT account_id, chat_id, message_id, sender, sender_jid, from_me,
                            body, timestamp, message_type, source
                     FROM wa_messages
                     WHERE account_id = ?1
                       AND chat_id    = ?2
                       AND (body LIKE ?3 ESCAPE '\\' OR sender LIKE ?3 ESCAPE '\\')
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
                       AND (body LIKE ?2 ESCAPE '\\' OR sender LIKE ?2 ESCAPE '\\')
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
                       AND (body LIKE ?2 ESCAPE '\\' OR sender LIKE ?2 ESCAPE '\\')
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
                     WHERE body LIKE ?1 ESCAPE '\\' OR sender LIKE ?1 ESCAPE '\\'
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

/// Append `suffix` to the *file name* of `path` (so `whatsapp_data.db` + `-wal`
/// = `whatsapp_data.db-wal`, and `whatsapp_data.db` + `.corrupt-123` =
/// `whatsapp_data.db.corrupt-123`). SQLite names its side-files this way (not
/// as a new extension), and the quarantine keeps the corrupt image alongside
/// the original for inspection.
fn with_name_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.to_path_buf();
    let name = p
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    p.set_file_name(format!("{name}{suffix}"));
    p
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
#[path = "store_tests.rs"]
mod store_tests;

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
    fn search_messages_matches_sender_name() {
        // Person-name queries ("what did Alice say") only return rows when
        // search also looks at the `sender` column, because the sender's own
        // name almost never appears in the message body.
        let (store, _tmp) = make_store();
        let mut chats = HashMap::new();
        chats.insert(
            "chat-alice@c.us".to_string(),
            ChatMeta {
                name: Some("Alice Q".to_string()),
            },
        );
        store.upsert_chats("acct1", &chats).unwrap();

        let msgs = vec![
            IngestMessage {
                message_id: "alice-1".to_string(),
                chat_id: "chat-alice@c.us".to_string(),
                sender: Some("Alice".to_string()),
                sender_jid: Some("alice@c.us".to_string()),
                from_me: Some(false),
                // Body has no "Alice" — match must come from the sender column.
                body: Some("running 5 minutes late".to_string()),
                timestamp: Some(1_700_001_000),
                message_type: None,
                source: Some("cdp-dom".to_string()),
            },
            IngestMessage {
                message_id: "me-1".to_string(),
                chat_id: "chat-alice@c.us".to_string(),
                sender: Some("me".to_string()),
                sender_jid: None,
                from_me: Some(true),
                body: Some("no problem".to_string()),
                timestamp: Some(1_700_001_100),
                message_type: None,
                source: Some("cdp-dom".to_string()),
            },
        ];
        store.upsert_messages("acct1", &msgs).unwrap();

        let req = SearchMessagesRequest {
            query: "Alice".to_string(),
            chat_id: None,
            account_id: None,
            limit: None,
        };
        let results = store.search_messages(&req).unwrap();
        assert_eq!(results.len(), 1, "expected sender-name match: {results:?}");
        assert_eq!(results[0].sender, "Alice");
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
