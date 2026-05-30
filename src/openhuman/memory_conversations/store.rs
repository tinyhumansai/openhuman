//! JSONL-backed thread and message store. Thread metadata lives in
//! `threads.jsonl` (append-only upsert/delete log); each thread's messages
//! are appended to a per-thread JSONL file under `threads/<id>.jsonl`.
//!
//! All on-disk mutations serialise through a single process-wide mutex so
//! concurrent RPC handlers don't interleave writes.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tempfile::NamedTempFile;

use super::inverted_index::InvertedIndex;
use super::types::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
    CrossThreadHit,
};

const LOG_PREFIX: &str = "[memory:conversations]";
const THREADS_FILENAME: &str = "threads.jsonl";
const THREAD_MESSAGES_DIR: &str = "threads";
static CONVERSATION_STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Per-workspace inverted index cache. Keyed by the workspace's
/// `memory/conversations` root so multiple `ConversationStore` clones
/// pointing at the same workspace share one index. The cache outlives
/// individual store handles (which are cloneable PathBuf wrappers); it
/// is bounded by the number of distinct workspaces a single process
/// touches, which in practice is one. Tests using `TempDir` paths leave
/// behind dead entries when the dir is removed — acceptable for an
/// in-process cache.
///
/// # Lock ordering
///
/// When BOTH `CONVERSATION_STORE_LOCK` and `CONVERSATION_INDEX_CACHE`
/// must be held simultaneously, `CONVERSATION_STORE_LOCK` MUST be
/// acquired first. This applies to `append_message` (writes JSONL then
/// updates the warm index) and `with_index` (caller holds the outer
/// lock, then takes the cache lock to run the search closure).
///
/// `prime_index_if_cold` minimises shared locking. It may hold both
/// locks only momentarily, and always in the `CONVERSATION_STORE_LOCK`
/// → `CONVERSATION_INDEX_CACHE` order above: while holding the outer
/// lock to snapshot live thread IDs via `thread_index_unlocked`
/// (header-only, no per-thread I/O) it re-checks the cache once. It then
/// releases `CONVERSATION_STORE_LOCK` before reading per-thread JSONL
/// content (no lock held) and finally acquires `CONVERSATION_INDEX_CACHE`
/// alone to insert the built index. It never holds both across the slow
/// JSONL walk, and neither operation calls back into a function that
/// would acquire the other lock.
///
/// `list_threads_unlocked` MUST NOT be used inside the locked snapshot —
/// it calls `measure_messages_unlocked` per legacy thread (no Stats
/// history), which reads every per-thread JSONL file and appends a
/// `Stats` entry to `threads.jsonl`, reintroducing the multi-second
/// stall under the outer lock that this design was built to avoid.
static CONVERSATION_INDEX_CACHE: Lazy<Mutex<HashMap<PathBuf, InvertedIndex>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn redact_title_for_log(title: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    format!(
        "<redacted len={} hash={:016x}>",
        title.chars().count(),
        hasher.finish()
    )
}

/// Counts returned by [`purge_threads`] — how much was deleted.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConversationPurgeStats {
    pub thread_count: usize,
    pub message_count: usize,
}

/// Workspace-rooted handle that reads and writes the JSONL conversation log.
#[derive(Debug, Clone)]
pub struct ConversationStore {
    workspace_dir: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ThreadLogEntry {
    Upsert {
        thread_id: String,
        title: String,
        created_at: String,
        updated_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_thread_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        labels: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        personality_id: Option<String>,
    },
    Delete {
        thread_id: String,
        deleted_at: String,
    },
    /// Single message appended to a thread. Increments `message_count` by 1
    /// and overwrites `last_message_at`. Emitted by `append_message` to keep
    /// list_threads O(threads.jsonl) instead of O(total messages).
    MessageAppended {
        thread_id: String,
        last_message_at: String,
    },
    /// Absolute stat snapshot — overrides the running count + timestamp.
    /// Used to backfill legacy threads whose messages were written before
    /// `MessageAppended` existed.
    Stats {
        thread_id: String,
        message_count: usize,
        last_message_at: String,
    },
}

impl ConversationStore {
    /// Construct a store rooted at the given workspace directory.
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    /// Create or update a thread, appending an `Upsert` entry to `threads.jsonl`.
    pub fn ensure_thread(
        &self,
        request: CreateConversationThread,
    ) -> Result<ConversationThread, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        let root = self.ensure_root()?;
        let threads_path = root.join(THREADS_FILENAME);
        let now = request.created_at.clone();
        append_jsonl(
            &threads_path,
            &ThreadLogEntry::Upsert {
                thread_id: request.id.clone(),
                title: request.title.clone(),
                created_at: request.created_at.clone(),
                updated_at: now,
                parent_thread_id: request.parent_thread_id.clone(),
                labels: request.labels.clone(),
                personality_id: request.personality_id.clone(),
            },
        )?;
        debug!(
            "{LOG_PREFIX} ensured thread id={} path={}",
            request.id,
            threads_path.display()
        );
        self.thread_summary_unlocked(&request.id)?
            .ok_or_else(|| format!("thread {} missing after ensure", request.id))
    }

    /// List all live threads (folding the upsert/delete log).
    pub fn list_threads(&self) -> Result<Vec<ConversationThread>, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        self.list_threads_unlocked()
    }

    /// Read every persisted message for a thread in append order.
    pub fn get_messages(&self, thread_id: &str) -> Result<Vec<ConversationMessage>, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        if !self.thread_exists_unlocked(thread_id)? {
            return Ok(Vec::new());
        }
        let path = self.thread_messages_path(thread_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        read_jsonl::<ConversationMessage>(&path)
    }

    /// Substring-match messages across **every** thread in the workspace,
    /// optionally excluding one thread (the active chat). Returns up to
    /// `limit` of the most-recent matching messages, newest first.
    ///
    /// Workspace scope is enforced by the store's `workspace_dir` — one
    /// workspace dir per user — so this helper cannot cross that
    /// boundary. Issue #1505: the conversational durable-fact pipeline
    /// (`learning::transcript_ingest`) is async and batched, so cross-
    /// chat continuity needs a direct cross-thread reader to surface
    /// context the user shared in chat A when they ask a dependent
    /// question in chat B.
    ///
    /// Backed by an in-memory trigram/CJK-bigram inverted index
    /// (`super::inverted_index`). The legacy implementation walked every
    /// JSONL file and did `content.to_lowercase().contains(term)` per
    /// message, which is O(threads × messages × content_len). The index
    /// turns that into O(|posting lists|) for typical queries while
    /// preserving the previous scoring contract
    /// (`score = matched_terms / total_terms`, recency tiebreak).
    ///
    /// # Lock strategy (issue #2849)
    ///
    /// **Fast path (warm cache):** acquires only `CONVERSATION_INDEX_CACHE`
    /// — no outer store lock — and returns immediately.
    ///
    /// **Cold path (first access):** snapshots the thread list under
    /// `CONVERSATION_STORE_LOCK` (brief), then releases it before reading
    /// JSONL files to build the inverted index. This avoids blocking
    /// `append_message` / `get_messages` / `list_threads` during the
    /// potentially-long rebuild. JSONL files are append-only, so a
    /// concurrent write during the rebuild may mean the rebuilt index
    /// misses that one message. It is *not* re-read later; subsequent
    /// `append_message` calls only index their own (new) messages once
    /// the cache is warm. The missed message therefore stays absent
    /// until the cache is evicted and rebuilt — an accepted tradeoff
    /// for issue #2849.
    pub fn search_cross_thread_messages(
        &self,
        query: &str,
        limit: usize,
        exclude_thread_id: Option<&str>,
    ) -> Result<Vec<CrossThreadHit>, String> {
        // Warm the index outside the outer lock so concurrent
        // append_message / get_messages calls are not stalled during the
        // cold JSONL rebuild (which can take seconds on large workspaces).
        // After this returns the cache entry is guaranteed to exist, so
        // with_index will not trigger a second rebuild.
        self.prime_index_if_cold()?;

        let _guard = CONVERSATION_STORE_LOCK.lock();
        self.with_index(|idx| idx.search(query, limit, exclude_thread_id))
    }

    /// If no index entry exists for this workspace, snapshot the live thread
    /// IDs under `CONVERSATION_STORE_LOCK` (fast — reads only
    /// `threads.jsonl`, no per-thread I/O), release that lock, read all
    /// per-thread JSONL files with no lock held (safe — append-only), then
    /// insert the built index into `CONVERSATION_INDEX_CACHE` using
    /// `entry().or_insert()` so a concurrent prime that finished first wins
    /// and ours is discarded.
    ///
    /// After this call returns, `with_index` will always find a warm entry
    /// and will not re-enter `populate_index_unlocked`.
    fn prime_index_if_cold(&self) -> Result<(), String> {
        let key = self.root_dir();
        // Fast path: already warm — one tiny lock acquisition and out.
        if CONVERSATION_INDEX_CACHE.lock().contains_key(&key) {
            return Ok(());
        }
        // Snapshot live thread IDs while holding the outer lock.
        // `thread_index_unlocked` reads only `threads.jsonl` (header-only,
        // O(threads), no per-thread file I/O) — the lock is released
        // immediately after, so the slow content reads below never block
        // concurrent writers.
        //
        // Do NOT call `list_threads_unlocked` here.  For workspaces where
        // any thread has no `MessageAppended`/`Stats` history (common before
        // the Stats log was introduced), `list_threads_unlocked` triggers
        // `measure_messages_unlocked` + a `Stats` append per thread — all
        // under `CONVERSATION_STORE_LOCK` — reintroducing the multi-second
        // stall this function is designed to avoid.
        let thread_ids: Vec<String> = {
            let _guard = CONVERSATION_STORE_LOCK.lock();
            // Re-check after acquiring: a concurrent prime may have just
            // finished while we waited for the outer lock.
            if CONVERSATION_INDEX_CACHE.lock().contains_key(&key) {
                return Ok(());
            }
            self.thread_index_unlocked()?.into_keys().collect()
        };
        // Build the index with no locks held.  The per-thread JSONL files are
        // append-only so reads are safe without synchronisation. The worst
        // case is a message appended during this window: append_message sees a
        // still-cold cache (we have not inserted yet) and skips its index
        // update, so that specific message stays absent from the in-memory
        // index until the next cold rebuild (e.g. a process restart re-reads
        // JSONL). Later appends index only their own messages, not the raced
        // one. This is the accepted tradeoff documented in issue #2849.
        let mut idx = InvertedIndex::new();
        for thread_id in &thread_ids {
            let path = self.thread_messages_path(thread_id);
            if !path.exists() {
                continue;
            }
            match read_jsonl::<ConversationMessage>(&path) {
                Ok(messages) => {
                    for msg in messages {
                        idx.insert(thread_id, msg);
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "{LOG_PREFIX} index prime skipped unreadable file path={} error={}",
                        path.display(),
                        err
                    );
                }
            }
        }
        // Insert only if the key is still absent — a concurrent prime that
        // finished first wins; ours is discarded. Log the discard so
        // duplicate-build churn under load is diagnosable.
        {
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            if cache.contains_key(&key) {
                debug!(
                    "{LOG_PREFIX} discarded freshly-built index; concurrent prime won workspace={}",
                    key.display()
                );
            } else {
                cache.insert(key.clone(), idx);
                debug!(
                    "{LOG_PREFIX} inverted index primed workspace={}",
                    key.display()
                );
            }
        }
        Ok(())
    }

    /// Acquire the cached inverted index for this workspace (building it
    /// from JSONL on first access) and run `f` against it. Caller MUST
    /// hold `CONVERSATION_STORE_LOCK` for the duration of the closure.
    ///
    /// In the normal path the index has already been warmed by
    /// `prime_index_if_cold`, so the cold-build branch here is a safety net
    /// for any future callers that bypass the priming step.
    fn with_index<R>(&self, f: impl FnOnce(&mut InvertedIndex) -> R) -> Result<R, String> {
        let key = self.root_dir();
        let mut cache = CONVERSATION_INDEX_CACHE.lock();
        if !cache.contains_key(&key) {
            let mut idx = InvertedIndex::new();
            self.populate_index_unlocked(&mut idx)?;
            cache.insert(key.clone(), idx);
        }
        let idx = cache.get_mut(&key).expect("inserted above if absent");
        Ok(f(idx))
    }

    /// Walk every per-thread JSONL file in the workspace and insert each
    /// message into `idx`. Used as the fallback cold-build path inside
    /// `with_index`; `prime_index_if_cold` handles the normal first-access
    /// case outside the outer lock. The JSONL files are the source of truth
    /// so a rebuild after a process crash is always safe.
    fn populate_index_unlocked(&self, idx: &mut InvertedIndex) -> Result<(), String> {
        // Caller (`with_index`) already holds `CONVERSATION_STORE_LOCK`, so we
        // must NOT re-acquire it here — `parking_lot::Mutex` is not reentrant
        // and doing so would deadlock. Use the `_unlocked` thread reader
        // directly.
        //
        // `list_threads_unlocked` already handles a fresh workspace:
        // `ensure_root` creates the directory + threads log if missing, and
        // `read_jsonl` returns an empty Vec for an empty file. Anything that
        // still bubbles up here is a real filesystem/setup failure and must
        // propagate — silently returning Ok would mask it and make search
        // appear to return zero results for an undiagnosed reason.
        let threads = self.list_threads_unlocked()?;
        for thread in threads {
            let path = self.thread_messages_path(&thread.id);
            if !path.exists() {
                continue;
            }
            let messages = match read_jsonl::<ConversationMessage>(&path) {
                Ok(m) => m,
                Err(err) => {
                    tracing::warn!(
                        "{LOG_PREFIX} index build skipped unreadable file path={} error={}",
                        path.display(),
                        err
                    );
                    continue;
                }
            };
            for msg in messages {
                idx.insert(&thread.id, msg);
            }
        }
        debug!(
            "{LOG_PREFIX} inverted index populated workspace={}",
            self.root_dir().display()
        );
        Ok(())
    }

    /// Append a message to the thread's JSONL file. Errors if the thread is missing.
    pub fn append_message(
        &self,
        thread_id: &str,
        message: ConversationMessage,
    ) -> Result<ConversationMessage, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        if !self.thread_exists_unlocked(thread_id)? {
            return Err(format!("thread {} not found", thread_id));
        }
        let path = self.thread_messages_path(thread_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create conversation dir {}: {e}", parent.display()))?;
        }
        append_jsonl(&path, &message)?;
        // Bump the threads-log stat trail so subsequent `list_threads`
        // calls can compute (message_count, last_message_at) without
        // re-reading this file.
        let threads_path = self.root_dir().join(THREADS_FILENAME);
        append_jsonl(
            &threads_path,
            &ThreadLogEntry::MessageAppended {
                thread_id: thread_id.to_string(),
                last_message_at: message.created_at.clone(),
            },
        )?;
        // Keep the inverted index in sync. We only update if the index
        // has already been materialized for this workspace — otherwise
        // the next search will lazily rebuild and pick up this message
        // anyway, and we avoid paying the rebuild cost on a write path.
        // `insert` takes the message by value so it can move owned
        // fields straight into its `DocEntry`; we clone here only when
        // the cache is actually warm, paying for one extra owned copy
        // (instead of cloning each field inside `insert`).
        {
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            if let Some(idx) = cache.get_mut(&self.root_dir()) {
                idx.insert(thread_id, message.clone());
            }
        }
        debug!(
            "{LOG_PREFIX} appended message thread_id={} message_id={} path={}",
            thread_id,
            message.id,
            path.display()
        );
        Ok(message)
    }

    /// Rewrite the thread title via a new `Upsert` log entry, preserving labels.
    pub fn update_thread_title(
        &self,
        thread_id: &str,
        title: &str,
        updated_at: &str,
    ) -> Result<ConversationThread, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        let index = self.thread_index_unlocked()?;
        let entry = index
            .get(thread_id)
            .ok_or_else(|| format!("thread {} not found", thread_id))?;
        let threads_path = self.ensure_root()?.join(THREADS_FILENAME);
        append_jsonl(
            &threads_path,
            &ThreadLogEntry::Upsert {
                thread_id: thread_id.to_string(),
                title: title.to_string(),
                created_at: entry.created_at.clone(),
                updated_at: updated_at.to_string(),
                parent_thread_id: entry.parent_thread_id.clone(),
                labels: Some(entry.labels.clone()),
                personality_id: entry.personality_id.clone(),
            },
        )?;
        debug!(
            "{LOG_PREFIX} updated thread title id={} title={} path={}",
            thread_id,
            redact_title_for_log(title),
            threads_path.display()
        );
        self.thread_summary_unlocked(thread_id)?
            .ok_or_else(|| format!("thread {} missing after title update", thread_id))
    }

    /// Replace the label set on a thread via a new `Upsert` log entry.
    pub fn update_thread_labels(
        &self,
        thread_id: &str,
        labels: Vec<String>,
        updated_at: &str,
    ) -> Result<ConversationThread, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        let index = self.thread_index_unlocked()?;
        let entry = index
            .get(thread_id)
            .ok_or_else(|| format!("thread {} not found", thread_id))?;
        let threads_path = self.ensure_root()?.join(THREADS_FILENAME);
        append_jsonl(
            &threads_path,
            &ThreadLogEntry::Upsert {
                thread_id: thread_id.to_string(),
                title: entry.title.clone(),
                created_at: entry.created_at.clone(),
                updated_at: updated_at.to_string(),
                parent_thread_id: entry.parent_thread_id.clone(),
                labels: Some(labels),
                personality_id: entry.personality_id.clone(),
            },
        )?;
        debug!(
            "{LOG_PREFIX} updated thread labels id={} path={}",
            thread_id,
            threads_path.display()
        );
        self.thread_summary_unlocked(thread_id)?
            .ok_or_else(|| format!("thread {} missing after labels update", thread_id))
    }

    /// Apply a patch to one message and rewrite the thread's JSONL file in place.
    pub fn update_message(
        &self,
        thread_id: &str,
        message_id: &str,
        patch: ConversationMessagePatch,
    ) -> Result<ConversationMessage, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        let path = self.thread_messages_path(thread_id);
        let mut messages = read_jsonl::<ConversationMessage>(&path)?;
        let mut updated: Option<ConversationMessage> = None;
        for message in &mut messages {
            if message.id == message_id {
                if let Some(extra_metadata) = patch.extra_metadata.clone() {
                    message.extra_metadata = extra_metadata;
                }
                updated = Some(message.clone());
                break;
            }
        }
        let updated = updated
            .ok_or_else(|| format!("message {} not found in thread {}", message_id, thread_id))?;
        rewrite_jsonl(&path, &messages)?;
        debug!(
            "{LOG_PREFIX} updated message thread_id={} message_id={} path={}",
            thread_id,
            message_id,
            path.display()
        );
        Ok(updated)
    }

    /// Append a `Delete` entry and remove the thread's messages file. Returns
    /// `false` if the thread did not exist.
    pub fn delete_thread(&self, thread_id: &str, deleted_at: &str) -> Result<bool, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        if !self.thread_exists_unlocked(thread_id)? {
            return Ok(false);
        }
        let root = self.ensure_root()?;
        let threads_path = root.join(THREADS_FILENAME);
        append_jsonl(
            &threads_path,
            &ThreadLogEntry::Delete {
                thread_id: thread_id.to_string(),
                deleted_at: deleted_at.to_string(),
            },
        )?;
        let messages_path = self.thread_messages_path(thread_id);
        match fs::remove_file(&messages_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "delete conversation messages {}: {error}",
                    messages_path.display()
                ));
            }
        }
        // Drop every indexed message for this thread so future searches
        // don't surface stale content.
        {
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            if let Some(idx) = cache.get_mut(&self.root_dir()) {
                idx.remove_thread(thread_id);
            }
        }
        debug!(
            "{LOG_PREFIX} deleted thread id={} path={}",
            thread_id,
            messages_path.display()
        );
        Ok(true)
    }

    /// Wipe the entire conversation directory and re-create an empty layout.
    pub fn purge_threads(&self) -> Result<ConversationPurgeStats, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        let stats = self.purge_stats_unlocked()?;
        let root = self.root_dir();
        if root.exists() {
            fs::remove_dir_all(&root)
                .map_err(|e| format!("remove conversation dir {}: {e}", root.display()))?;
        }
        self.ensure_root()?;
        // Drop the cached inverted index — the workspace is now empty,
        // and any next search will lazily rebuild from the (now empty)
        // JSONL tree.
        {
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            cache.remove(&root);
        }
        debug!(
            "{LOG_PREFIX} purged threads={} messages={} root={}",
            stats.thread_count,
            stats.message_count,
            root.display()
        );
        Ok(stats)
    }

    fn ensure_root(&self) -> Result<PathBuf, String> {
        let root = self.root_dir();
        let threads_dir = root.join(THREAD_MESSAGES_DIR);
        fs::create_dir_all(&threads_dir)
            .map_err(|e| format!("create conversation dir {}: {e}", threads_dir.display()))?;
        let threads_file = root.join(THREADS_FILENAME);
        if !threads_file.exists() {
            File::create(&threads_file)
                .map_err(|e| format!("create threads log {}: {e}", threads_file.display()))?;
        }
        Ok(root)
    }

    fn root_dir(&self) -> PathBuf {
        self.workspace_dir.join("memory").join("conversations")
    }

    fn thread_messages_path(&self, thread_id: &str) -> PathBuf {
        self.root_dir()
            .join(THREAD_MESSAGES_DIR)
            .join(format!("{}.jsonl", hex::encode(thread_id.as_bytes())))
    }

    fn list_threads_unlocked(&self) -> Result<Vec<ConversationThread>, String> {
        let mut index = self.thread_index_unlocked()?;
        // Backfill stats for any thread with no MessageAppended/Stats history
        // yet (legacy data). The slow per-thread file read happens at most
        // once per thread — we persist a `Stats` snapshot so subsequent
        // list_threads calls hit the fast path.
        let needs_backfill: Vec<String> = index
            .iter()
            .filter_map(|(id, entry)| {
                if entry.message_count.is_none() {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        if !needs_backfill.is_empty() {
            let threads_path = self.ensure_root()?.join(THREADS_FILENAME);
            for thread_id in &needs_backfill {
                let (count, last_message_at) = self.measure_messages_unlocked(thread_id)?;
                // Treat created_at as last_message_at when there are no
                // messages — keeps the sort key meaningful and matches the
                // pre-refactor semantics.
                let resolved_last = last_message_at.unwrap_or_else(|| {
                    index
                        .get(thread_id)
                        .map(|e| e.created_at.clone())
                        .unwrap_or_default()
                });
                append_jsonl(
                    &threads_path,
                    &ThreadLogEntry::Stats {
                        thread_id: thread_id.clone(),
                        message_count: count,
                        last_message_at: resolved_last.clone(),
                    },
                )?;
                if let Some(entry) = index.get_mut(thread_id) {
                    entry.message_count = Some(count);
                    entry.last_message_at = Some(resolved_last);
                }
                debug!(
                    "{LOG_PREFIX} backfilled stats thread_id={} count={}",
                    thread_id, count
                );
            }
        }

        let mut threads: Vec<ConversationThread> = index
            .iter()
            .map(|(thread_id, entry)| {
                let message_count = entry.message_count.unwrap_or(0);
                let last_message_at = entry
                    .last_message_at
                    .clone()
                    .unwrap_or_else(|| entry.created_at.clone());
                ConversationThread {
                    id: thread_id.clone(),
                    title: entry.title.clone(),
                    chat_id: None,
                    is_active: true,
                    message_count,
                    last_message_at,
                    created_at: entry.created_at.clone(),
                    parent_thread_id: entry.parent_thread_id.clone(),
                    labels: entry.labels.clone(),
                    personality_id: entry.personality_id.clone(),
                }
            })
            .collect();
        threads.sort_by(|a, b| {
            b.last_message_at
                .cmp(&a.last_message_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        Ok(threads)
    }

    /// Count messages and find the newest timestamp by reading the
    /// per-thread JSONL file. Slow path — used only when the threads-log
    /// stat history is missing (legacy data) so we can write a one-time
    /// `Stats` snapshot.
    fn measure_messages_unlocked(
        &self,
        thread_id: &str,
    ) -> Result<(usize, Option<String>), String> {
        let path = self.thread_messages_path(thread_id);
        if !path.exists() {
            return Ok((0, None));
        }
        let messages = read_jsonl::<ConversationMessage>(&path)?;
        let count = messages.len();
        let last = messages.last().map(|m| m.created_at.clone());
        Ok((count, last))
    }

    fn thread_summary_unlocked(
        &self,
        thread_id: &str,
    ) -> Result<Option<ConversationThread>, String> {
        let index = self.thread_index_unlocked()?;
        let entry = match index.get(thread_id) {
            Some(entry) => entry,
            None => return Ok(None),
        };
        // Prefer the index-tracked stats (cheap). Fall back to a single
        // per-thread file read for legacy threads with no stat history —
        // list_threads is responsible for permanently backfilling those.
        let (message_count, last_message_at) =
            match (entry.message_count, entry.last_message_at.as_ref()) {
                (Some(count), Some(last_at)) => (count, last_at.clone()),
                _ => {
                    let (count, last_at) = self.measure_messages_unlocked(thread_id)?;
                    (count, last_at.unwrap_or_else(|| entry.created_at.clone()))
                }
            };
        Ok(Some(ConversationThread {
            id: thread_id.to_string(),
            title: entry.title.clone(),
            chat_id: None,
            is_active: true,
            message_count,
            last_message_at,
            created_at: entry.created_at.clone(),
            parent_thread_id: entry.parent_thread_id.clone(),
            labels: entry.labels.clone(),
            personality_id: entry.personality_id.clone(),
        }))
    }

    fn thread_exists_unlocked(&self, thread_id: &str) -> Result<bool, String> {
        Ok(self.thread_index_unlocked()?.contains_key(thread_id))
    }

    fn thread_index_unlocked(&self) -> Result<BTreeMap<String, ThreadIndexEntry>, String> {
        self.ensure_root()?;
        let path = self.root_dir().join(THREADS_FILENAME);
        let mut index: BTreeMap<String, ThreadIndexEntry> = BTreeMap::new();
        for entry in read_jsonl::<ThreadLogEntry>(&path)? {
            match entry {
                ThreadLogEntry::Upsert {
                    thread_id,
                    title,
                    created_at,
                    parent_thread_id,
                    labels,
                    personality_id,
                    ..
                } => {
                    let (
                        created_at_value,
                        parent_thread_id_value,
                        labels_value,
                        message_count_value,
                        last_message_at_value,
                        personality_id_value,
                    ) = match index.get(&thread_id) {
                        Some(existing) => (
                            existing.created_at.clone(),
                            parent_thread_id.or_else(|| existing.parent_thread_id.clone()),
                            labels.unwrap_or_else(|| existing.labels.clone()),
                            existing.message_count,
                            existing.last_message_at.clone(),
                            personality_id.or_else(|| existing.personality_id.clone()),
                        ),
                        None => {
                            let inferred = labels.unwrap_or_else(|| infer_labels(&thread_id));
                            (
                                created_at,
                                parent_thread_id,
                                inferred,
                                None,
                                None,
                                personality_id,
                            )
                        }
                    };
                    index.insert(
                        thread_id,
                        ThreadIndexEntry {
                            title,
                            created_at: created_at_value,
                            parent_thread_id: parent_thread_id_value,
                            labels: labels_value,
                            message_count: message_count_value,
                            last_message_at: last_message_at_value,
                            personality_id: personality_id_value,
                        },
                    );
                }
                ThreadLogEntry::Delete { thread_id, .. } => {
                    index.remove(&thread_id);
                }
                ThreadLogEntry::MessageAppended {
                    thread_id,
                    last_message_at,
                } => {
                    if let Some(entry) = index.get_mut(&thread_id) {
                        // Increment from a known baseline. If we have no
                        // baseline yet (legacy thread with messages but no
                        // Stats snapshot), leave count as `None` so the
                        // backfill path in `list_threads_unlocked` can do
                        // the one-shot file read instead of producing a
                        // wrong "1" here.
                        if let Some(count) = entry.message_count.as_mut() {
                            *count += 1;
                        }
                        entry.last_message_at = Some(last_message_at);
                    }
                }
                ThreadLogEntry::Stats {
                    thread_id,
                    message_count,
                    last_message_at,
                } => {
                    if let Some(entry) = index.get_mut(&thread_id) {
                        entry.message_count = Some(message_count);
                        entry.last_message_at = Some(last_message_at);
                    }
                }
            }
        }
        Ok(index)
    }

    fn purge_stats_unlocked(&self) -> Result<ConversationPurgeStats, String> {
        let threads = self.list_threads_unlocked()?;
        let message_count = threads.iter().map(|thread| thread.message_count).sum();
        Ok(ConversationPurgeStats {
            thread_count: threads.len(),
            message_count,
        })
    }
}

#[derive(Debug, Clone)]
struct ThreadIndexEntry {
    title: String,
    created_at: String,
    parent_thread_id: Option<String>,
    labels: Vec<String>,
    /// Folded message count. `None` means we have no `MessageAppended` /
    /// `Stats` history for this thread yet (legacy data) — `list_threads`
    /// backfills by doing a one-shot read of the per-thread messages file.
    message_count: Option<usize>,
    /// Timestamp of the newest message, or `None` if unknown (legacy).
    last_message_at: Option<String>,
    personality_id: Option<String>,
}

fn infer_labels(thread_id: &str) -> Vec<String> {
    if thread_id == "proactive:morning_briefing" {
        vec!["briefing".to_string()]
    } else if thread_id.starts_with("proactive:") {
        vec!["notification".to_string()]
    } else {
        vec!["work".to_string()]
    }
}

fn read_jsonl<T>(path: &Path) -> Result<Vec<T>, String>
where
    T: for<'de> serde::Deserialize<'de>,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|e| format!("read {} line {}: {e}", path.display(), line_no + 1))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(trimmed) {
            Ok(value) => items.push(value),
            Err(error) => {
                warn!(
                    "{LOG_PREFIX} skipping invalid jsonl line path={} line={} error={}",
                    path.display(),
                    line_no + 1,
                    error
                );
            }
        }
    }
    Ok(items)
}

fn append_jsonl<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: serde::Serialize,
{
    let parent = path
        .parent()
        .ok_or_else(|| format!("resolve parent dir for {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("create jsonl dir {}: {e}", parent.display()))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {} for append: {e}", path.display()))?;
    let line = serde_json::to_string(value)
        .map_err(|e| format!("serialize jsonl line for {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("write {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("sync {}: {e}", path.display()))?;
    Ok(())
}

fn rewrite_jsonl<T>(path: &Path, values: &[T]) -> Result<(), String>
where
    T: serde::Serialize,
{
    let parent = path
        .parent()
        .ok_or_else(|| format!("resolve parent dir for {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("create jsonl dir {}: {e}", parent.display()))?;
    let mut temp = NamedTempFile::new_in(parent)
        .map_err(|e| format!("create temp jsonl in {}: {e}", parent.display()))?;
    for value in values {
        let line = serde_json::to_string(value)
            .map_err(|e| format!("serialize jsonl line for {}: {e}", path.display()))?;
        writeln!(temp, "{line}")
            .map_err(|e| format!("write temp jsonl for {}: {e}", path.display()))?;
    }
    temp.as_file_mut()
        .sync_all()
        .map_err(|e| format!("sync temp jsonl for {}: {e}", path.display()))?;
    temp.persist(path)
        .map_err(|e| format!("persist {}: {}", path.display(), e.error))?;
    Ok(())
}

/// Free-function shim around [`ConversationStore::ensure_thread`].
pub fn ensure_thread(
    workspace_dir: PathBuf,
    request: CreateConversationThread,
) -> Result<ConversationThread, String> {
    ConversationStore::new(workspace_dir).ensure_thread(request)
}

/// Free-function shim around [`ConversationStore::list_threads`].
pub fn list_threads(workspace_dir: PathBuf) -> Result<Vec<ConversationThread>, String> {
    ConversationStore::new(workspace_dir).list_threads()
}

/// Free-function shim around [`ConversationStore::get_messages`].
pub fn get_messages(
    workspace_dir: PathBuf,
    thread_id: &str,
) -> Result<Vec<ConversationMessage>, String> {
    ConversationStore::new(workspace_dir).get_messages(thread_id)
}

/// Free-function shim around [`ConversationStore::append_message`].
pub fn append_message(
    workspace_dir: PathBuf,
    thread_id: &str,
    message: ConversationMessage,
) -> Result<ConversationMessage, String> {
    ConversationStore::new(workspace_dir).append_message(thread_id, message)
}

/// Free-function shim around [`ConversationStore::update_thread_title`].
pub fn update_thread_title(
    workspace_dir: PathBuf,
    thread_id: &str,
    title: &str,
    updated_at: &str,
) -> Result<ConversationThread, String> {
    ConversationStore::new(workspace_dir).update_thread_title(thread_id, title, updated_at)
}

/// Free-function shim around [`ConversationStore::update_thread_labels`].
pub fn update_thread_labels(
    workspace_dir: PathBuf,
    thread_id: &str,
    labels: Vec<String>,
    updated_at: &str,
) -> Result<ConversationThread, String> {
    ConversationStore::new(workspace_dir).update_thread_labels(thread_id, labels, updated_at)
}

/// Free-function shim around [`ConversationStore::update_message`].
pub fn update_message(
    workspace_dir: PathBuf,
    thread_id: &str,
    message_id: &str,
    patch: ConversationMessagePatch,
) -> Result<ConversationMessage, String> {
    ConversationStore::new(workspace_dir).update_message(thread_id, message_id, patch)
}

/// Free-function shim around [`ConversationStore::purge_threads`].
pub fn purge_threads(workspace_dir: PathBuf) -> Result<ConversationPurgeStats, String> {
    ConversationStore::new(workspace_dir).purge_threads()
}

/// Free-function shim around [`ConversationStore::delete_thread`].
pub fn delete_thread(
    workspace_dir: PathBuf,
    thread_id: &str,
    deleted_at: &str,
) -> Result<bool, String> {
    ConversationStore::new(workspace_dir).delete_thread(thread_id, deleted_at)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
