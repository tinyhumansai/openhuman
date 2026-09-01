//! JSONL-backed thread and message store. Thread metadata lives in
//! `threads.jsonl` (append-only upsert/delete log); each thread's messages
//! are appended to a per-thread JSONL file under
//! `threads/<hex(thread_id)>.jsonl` so arbitrary provider ids remain
//! filesystem-safe.
//!
//! All on-disk mutations serialise through a single process-wide mutex so
//! concurrent RPC handlers don't interleave writes.
//!
//! This file is the store as it came back from the memory engine (#5560),
//! unchanged apart from the one constructor described below. The three
//! dependency substitutions the round trip introduced — [`std::sync::LazyLock`]
//! for the statics, the local [`hex_encode`] for per-thread filenames, and the
//! hand-rolled temp-write in [`rewrite_jsonl`] — are all kept: they produce the
//! bytes and the paths that every existing transcript already lives at, and a
//! move must not rewrite those. See [`super`]'s module docs for the full
//! accounting of what the trip cost.
//!
//! # The one thing that did change: `from_config`
//!
//! The engine's [`ConversationStore`] carried a second constructor,
//! `from_config(&MemoryConfig)`, whose whole body was
//! `Self::new(config.workspace.clone())`. It is gone rather than translated,
//! and that is the point of the move: it was this module's *only* coupling to
//! the rest of the engine, and re-expressing it here would either drag
//! `MemoryConfig` back in or add a second name for a constructor that already
//! exists. [`ConversationStore::new`] takes the workspace directory, every
//! caller in this host already has one, and no caller ever used `from_config`
//! — so nothing needed a translation and the derived on-disk root
//! (`<workspace>/memory/conversations`, see `ConversationStore::root_dir` in
//! `store_index.rs`) is byte-identical either way.
//!
//! # File split
//!
//! To respect the repo's file-size limit the `impl ConversationStore` is split
//! across two child modules — [`ops`] (the public CRUD + search API) and
//! [`index`] (private thread-folding and inverted-index helpers). Both are
//! descendant modules of `store`, so they share access to the private statics,
//! constants, log-entry enum, and JSONL helpers defined here.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use parking_lot::Mutex;
use uuid::Uuid;

use super::inverted_index::InvertedIndex;
use super::types::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
};

#[path = "store_ops.rs"]
mod ops;

#[path = "store_index.rs"]
mod index;

/// Filename of the append-only thread metadata log, relative to the
/// `memory/conversations` root.
pub(super) const THREADS_FILENAME: &str = "threads.jsonl";
/// Subdirectory (relative to the `memory/conversations` root) holding the
/// per-thread message JSONL files, named `<hex(thread_id)>.jsonl`.
pub(super) const THREAD_MESSAGES_DIR: &str = "threads";

/// Serialises every on-disk mutation so concurrent handlers can't interleave
/// writes to `threads.jsonl` or the per-thread message logs.
static CONVERSATION_STORE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
static CONVERSATION_INDEX_CACHE: LazyLock<Mutex<HashMap<PathBuf, InvertedIndex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Counts returned by [`ConversationStore::purge_threads`] — how much was deleted.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConversationPurgeStats {
    /// Number of threads removed.
    pub thread_count: usize,
    /// Total messages removed across all purged threads.
    pub message_count: usize,
}

/// Workspace-rooted handle that reads and writes the JSONL conversation log.
#[derive(Debug, Clone)]
pub struct ConversationStore {
    workspace_dir: PathBuf,
}

impl ConversationStore {
    /// Construct a store rooted at the given workspace directory.
    ///
    /// This is the only constructor. The conversation root is derived from
    /// `workspace_dir` alone (`<workspace>/memory/conversations`, see
    /// `root_dir` in `store_index.rs`), so the caller's workspace is the whole
    /// input and there is nothing for a config type to add.
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

/// One line in `threads.jsonl`. The append-only log is folded into the current
/// thread state by [`ConversationStore::thread_index_unlocked`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum ThreadLogEntry {
    /// Create-or-replace the metadata for `thread_id`. The latest `Upsert`
    /// wins when the log is folded; `op` wire string is `upsert`.
    ///
    /// NOTE (audit TR-8): `labels: Some(_)` always **replaces** the folded
    /// label set (see `thread_index_unlocked`'s `Upsert` handling) — it is
    /// not a merge. Callers that want to touch a thread without disturbing
    /// its labels (e.g. per-message thread upserts) must pass `labels: None`;
    /// `bus::persist_channel_turn` currently does not, and so resets a
    /// user-set label back to `["general"]` on every channel message.
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
    /// Tombstone removing `thread_id` from the folded state; `op` wire string
    /// is `delete`.
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

/// Folded current state for a single thread, derived from the log.
#[derive(Debug, Clone)]
pub(super) struct ThreadIndexEntry {
    /// Human-readable thread title from the latest `Upsert`.
    pub(super) title: String,
    /// Thread creation timestamp from the first `Upsert`.
    pub(super) created_at: String,
    /// Parent thread id for nested/threaded conversations, if any.
    pub(super) parent_thread_id: Option<String>,
    /// Folded labels (already normalised/inferred) bucketing this thread.
    pub(super) labels: Vec<String>,
    /// Folded message count. `None` means we have no `MessageAppended` /
    /// `Stats` history for this thread yet (legacy data) — `list_threads`
    /// backfills by doing a one-shot read of the per-thread messages file.
    pub(super) message_count: Option<usize>,
    /// Timestamp of the newest message, or `None` if unknown (legacy).
    pub(super) last_message_at: Option<String>,
    /// Personality/persona id bound to this thread, if any.
    pub(super) personality_id: Option<String>,
}

/// Default labels for a thread whose `Upsert` carried none, inferred from its
/// id namespace (proactive briefings/notifications vs ordinary chats).
pub(super) fn infer_labels(thread_id: &str) -> Vec<String> {
    if thread_id == "proactive:morning_briefing" {
        vec!["briefing".to_string()]
    } else if thread_id.starts_with("proactive:") {
        vec!["notification".to_string()]
    } else {
        vec!["general".to_string()]
    }
}

/// Canonicalise legacy label spellings into their current buckets and dedupe.
pub(super) fn normalize_labels(labels: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(labels.len());
    for label in labels {
        let next = match label.as_str() {
            "work" => "general".to_string(),
            "from_reflection" | "subconscious_tick" => "subconscious".to_string(),
            "agent-task" | "worker" => "tasks".to_string(),
            _ => label,
        };
        if !normalized.contains(&next) {
            normalized.push(next);
        }
    }
    normalized
}

/// Lowercase hex-encode bytes — used to derive a filesystem-safe per-thread
/// messages filename from an arbitrary thread id.
///
/// This exists because the memory engine had no `hex` dependency; this crate
/// does, so `hex::encode` would work here. It is kept anyway: this is the
/// function that decides which file a user's transcript is read from and
/// written to, and swapping it is only provably safe, never *obviously* safe.
/// The three lines are cheaper than the argument.
pub(super) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: [u8; 16] = *b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Read a JSONL file into a vector, skipping blank and invalid lines so a
/// single corrupt line never loses the rest of the transcript.
pub(super) fn read_jsonl<T>(path: &Path) -> Result<Vec<T>, String>
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
        if let Ok(value) = serde_json::from_str::<T>(trimmed) {
            items.push(value);
        }
    }
    Ok(items)
}

/// Append one serialized value as a JSONL line, fsync'd before returning.
pub(super) fn append_jsonl<T>(path: &Path, value: &T) -> Result<(), String>
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

/// Atomically rewrite `path` with `values`, one JSON object per line.
///
/// Writes to a sibling temp file then renames over the target so a crash
/// mid-write never leaves a partially-written transcript.
///
/// This hand-rolls what OpenHuman originally got from
/// `tempfile::NamedTempFile`, because the memory engine carried `tempfile` as
/// a dev-dependency only. This crate has it in full, so the substitution is no
/// longer forced — but reverting it would change the temp file's name, its
/// permissions, and which side deletes it when a write fails. That is a change
/// to the crash-safety path for a user's transcript, and it belongs in a
/// change that is about that, not in a module move.
pub(super) fn rewrite_jsonl<T>(path: &Path, values: &[T]) -> Result<(), String>
where
    T: serde::Serialize,
{
    let parent = path
        .parent()
        .ok_or_else(|| format!("resolve parent dir for {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("create jsonl dir {}: {e}", parent.display()))?;
    let tmp_path = parent.join(format!(".conversations-{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<(), String> {
        let mut temp = File::create(&tmp_path)
            .map_err(|e| format!("create temp jsonl in {}: {e}", parent.display()))?;
        for value in values {
            let line = serde_json::to_string(value)
                .map_err(|e| format!("serialize jsonl line for {}: {e}", path.display()))?;
            writeln!(temp, "{line}")
                .map_err(|e| format!("write temp jsonl for {}: {e}", path.display()))?;
        }
        temp.sync_all()
            .map_err(|e| format!("sync temp jsonl for {}: {e}", path.display()))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("persist {}: {error}", path.display()));
    }
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
