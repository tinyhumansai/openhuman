//! JSONL-backed thread and message store. Thread metadata lives in
//! `threads.jsonl` (append-only upsert/delete log); each thread's messages
//! are appended to a per-thread JSONL file under `threads/<id>.jsonl`.
//!
//! All on-disk mutations serialise through a single process-wide mutex so
//! concurrent RPC handlers don't interleave writes.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tempfile::NamedTempFile;

use super::types::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
};

const LOG_PREFIX: &str = "[memory:conversations]";
const THREADS_FILENAME: &str = "threads.jsonl";
const THREAD_MESSAGES_DIR: &str = "threads";
static CONVERSATION_STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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
    },
    Delete {
        thread_id: String,
        deleted_at: String,
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
        read_jsonl::<ConversationMessage>(&self.thread_messages_path(thread_id))
    }

    /// Append a message to the thread's JSONL file. Errors if the thread is missing.
    pub fn append_message(
        &self,
        thread_id: &str,
        message: ConversationMessage,
    ) -> Result<ConversationMessage, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        if !self.thread_exists_unlocked(thread_id)? {
            return Err(format!("thread {} does not exist", thread_id));
        }
        let path = self.thread_messages_path(thread_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create conversation dir {}: {e}", parent.display()))?;
        }
        append_jsonl(&path, &message)?;
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
            .ok_or_else(|| format!("thread {} does not exist", thread_id))?;
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
            .ok_or_else(|| format!("thread {} does not exist", thread_id))?;
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
        let index = self.thread_index_unlocked()?;
        let mut threads = Vec::with_capacity(index.len());
        for thread_id in index.keys() {
            if let Some(summary) = self.thread_summary_unlocked(thread_id)? {
                threads.push(summary);
            }
        }
        threads.sort_by(|a, b| {
            b.last_message_at
                .cmp(&a.last_message_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        Ok(threads)
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
        let messages = read_jsonl::<ConversationMessage>(&self.thread_messages_path(thread_id))?;
        let message_count = messages.len();
        let last_message_at = messages
            .last()
            .map(|message| message.created_at.clone())
            .unwrap_or_else(|| entry.created_at.clone());
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
                    ..
                } => {
                    let (created_at_value, parent_thread_id_value, labels_value) =
                        match index.get(&thread_id) {
                            Some(existing) => (
                                existing.created_at.clone(),
                                parent_thread_id.or_else(|| existing.parent_thread_id.clone()),
                                labels.unwrap_or_else(|| existing.labels.clone()),
                            ),
                            None => {
                                let inferred = labels.unwrap_or_else(|| infer_labels(&thread_id));
                                (created_at, parent_thread_id, inferred)
                            }
                        };
                    index.insert(
                        thread_id,
                        ThreadIndexEntry {
                            title,
                            created_at: created_at_value,
                            parent_thread_id: parent_thread_id_value,
                            labels: labels_value,
                        },
                    );
                }
                ThreadLogEntry::Delete { thread_id, .. } => {
                    index.remove(&thread_id);
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
