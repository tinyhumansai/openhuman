use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
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

#[derive(Debug, Clone, Copy, Default)]
pub struct ConversationPurgeStats {
    pub thread_count: usize,
    pub message_count: usize,
}

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
    },
    Delete {
        thread_id: String,
        deleted_at: String,
    },
}

impl ConversationStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

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

    pub fn list_threads(&self) -> Result<Vec<ConversationThread>, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        self.list_threads_unlocked()
    }

    pub fn get_messages(&self, thread_id: &str) -> Result<Vec<ConversationMessage>, String> {
        let _guard = CONVERSATION_STORE_LOCK.lock();
        if !self.thread_exists_unlocked(thread_id)? {
            return Ok(Vec::new());
        }
        read_jsonl::<ConversationMessage>(&self.thread_messages_path(thread_id))
    }

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
                    ..
                } => {
                    let created_at_value = match index.get(&thread_id) {
                        Some(existing) => existing.created_at.clone(),
                        None => created_at,
                    };
                    index.insert(
                        thread_id,
                        ThreadIndexEntry {
                            title,
                            created_at: created_at_value,
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

pub fn ensure_thread(
    workspace_dir: PathBuf,
    request: CreateConversationThread,
) -> Result<ConversationThread, String> {
    ConversationStore::new(workspace_dir).ensure_thread(request)
}

pub fn list_threads(workspace_dir: PathBuf) -> Result<Vec<ConversationThread>, String> {
    ConversationStore::new(workspace_dir).list_threads()
}

pub fn get_messages(
    workspace_dir: PathBuf,
    thread_id: &str,
) -> Result<Vec<ConversationMessage>, String> {
    ConversationStore::new(workspace_dir).get_messages(thread_id)
}

pub fn append_message(
    workspace_dir: PathBuf,
    thread_id: &str,
    message: ConversationMessage,
) -> Result<ConversationMessage, String> {
    ConversationStore::new(workspace_dir).append_message(thread_id, message)
}

pub fn update_message(
    workspace_dir: PathBuf,
    thread_id: &str,
    message_id: &str,
    patch: ConversationMessagePatch,
) -> Result<ConversationMessage, String> {
    ConversationStore::new(workspace_dir).update_message(thread_id, message_id, patch)
}

pub fn purge_threads(workspace_dir: PathBuf) -> Result<ConversationPurgeStats, String> {
    ConversationStore::new(workspace_dir).purge_threads()
}

pub fn delete_thread(
    workspace_dir: PathBuf,
    thread_id: &str,
    deleted_at: &str,
) -> Result<bool, String> {
    ConversationStore::new(workspace_dir).delete_thread(thread_id, deleted_at)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use serde_json::json;

    fn make_store() -> (TempDir, ConversationStore) {
        let temp = TempDir::new().expect("tempdir");
        let store = ConversationStore::new(temp.path().to_path_buf());
        (temp, store)
    }

    #[test]
    fn store_roundtrips_threads_and_messages() {
        let (_temp, store) = make_store();
        let created_at = "2026-04-10T12:00:00Z".to_string();
        let thread = store
            .ensure_thread(CreateConversationThread {
                id: "default-thread".to_string(),
                title: "Conversation".to_string(),
                created_at: created_at.clone(),
            })
            .expect("ensure thread");
        assert_eq!(thread.message_count, 0);

        store
            .append_message(
                "default-thread",
                ConversationMessage {
                    id: "m1".to_string(),
                    content: "hello".to_string(),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "user".to_string(),
                    created_at: "2026-04-10T12:01:00Z".to_string(),
                },
            )
            .expect("append message");

        let threads = store.list_threads().expect("list threads");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_count, 1);
        assert_eq!(threads[0].last_message_at, "2026-04-10T12:01:00Z");

        let messages = store.get_messages("default-thread").expect("get messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
    }

    #[test]
    fn store_updates_message_metadata() {
        let (_temp, store) = make_store();
        store
            .ensure_thread(CreateConversationThread {
                id: "default-thread".to_string(),
                title: "Conversation".to_string(),
                created_at: "2026-04-10T12:00:00Z".to_string(),
            })
            .expect("ensure thread");
        store
            .append_message(
                "default-thread",
                ConversationMessage {
                    id: "m1".to_string(),
                    content: "hello".to_string(),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "user".to_string(),
                    created_at: "2026-04-10T12:01:00Z".to_string(),
                },
            )
            .expect("append message");

        let updated = store
            .update_message(
                "default-thread",
                "m1",
                ConversationMessagePatch {
                    extra_metadata: Some(json!({ "myReactions": ["👍"] })),
                },
            )
            .expect("update message");

        assert_eq!(updated.extra_metadata, json!({ "myReactions": ["👍"] }));
        let messages = store.get_messages("default-thread").expect("get messages");
        assert_eq!(messages[0].extra_metadata, json!({ "myReactions": ["👍"] }));
    }

    #[test]
    fn purge_removes_threads_and_messages() {
        let (_temp, store) = make_store();
        store
            .ensure_thread(CreateConversationThread {
                id: "default-thread".to_string(),
                title: "Conversation".to_string(),
                created_at: "2026-04-10T12:00:00Z".to_string(),
            })
            .expect("ensure thread");
        store
            .append_message(
                "default-thread",
                ConversationMessage {
                    id: "m1".to_string(),
                    content: "hello".to_string(),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "user".to_string(),
                    created_at: "2026-04-10T12:01:00Z".to_string(),
                },
            )
            .expect("append message");

        let stats = store.purge_threads().expect("purge");
        assert_eq!(stats.thread_count, 1);
        assert_eq!(stats.message_count, 1);
        assert!(store.list_threads().expect("list threads").is_empty());
    }

    #[test]
    fn ensure_thread_is_idempotent() {
        let (_temp, store) = make_store();
        let req = CreateConversationThread {
            id: "t1".to_string(),
            title: "Thread".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
        };
        store.ensure_thread(req.clone()).unwrap();
        store.ensure_thread(req).unwrap();
        let threads = store.list_threads().unwrap();
        assert_eq!(threads.len(), 1);
    }

    #[test]
    fn delete_thread_removes_thread_and_messages() {
        let (_temp, store) = make_store();
        store
            .ensure_thread(CreateConversationThread {
                id: "t1".to_string(),
                title: "Thread".to_string(),
                created_at: "2026-04-10T12:00:00Z".to_string(),
            })
            .unwrap();
        store
            .append_message(
                "t1",
                ConversationMessage {
                    id: "m1".to_string(),
                    content: "msg".to_string(),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "user".to_string(),
                    created_at: "2026-04-10T12:01:00Z".to_string(),
                },
            )
            .unwrap();
        store.delete_thread("t1", "2026-04-10T12:02:00Z").unwrap();
        let threads = store.list_threads().unwrap();
        assert!(threads.is_empty());
    }

    #[test]
    fn delete_nonexistent_thread_is_ok() {
        let (_temp, store) = make_store();
        // Should not error
        store
            .delete_thread("nonexistent", "2026-04-10T12:00:00Z")
            .unwrap();
    }

    #[test]
    fn get_messages_empty_thread() {
        let (_temp, store) = make_store();
        store
            .ensure_thread(CreateConversationThread {
                id: "t1".to_string(),
                title: "Empty".to_string(),
                created_at: "2026-04-10T12:00:00Z".to_string(),
            })
            .unwrap();
        let messages = store.get_messages("t1").unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn get_messages_nonexistent_thread() {
        let (_temp, store) = make_store();
        let messages = store.get_messages("nonexistent").unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn multiple_threads_and_messages() {
        let (_temp, store) = make_store();
        for i in 0..3 {
            store
                .ensure_thread(CreateConversationThread {
                    id: format!("t{i}"),
                    title: format!("Thread {i}"),
                    created_at: format!("2026-04-10T12:0{i}:00Z"),
                })
                .unwrap();
            store
                .append_message(
                    &format!("t{i}"),
                    ConversationMessage {
                        id: format!("m{i}"),
                        content: format!("msg {i}"),
                        message_type: "text".to_string(),
                        extra_metadata: json!({}),
                        sender: "user".to_string(),
                        created_at: format!("2026-04-10T12:0{i}:30Z"),
                    },
                )
                .unwrap();
        }
        let threads = store.list_threads().unwrap();
        assert_eq!(threads.len(), 3);
    }

    #[test]
    fn purge_on_empty_store() {
        let (_temp, store) = make_store();
        let stats = store.purge_threads().unwrap();
        assert_eq!(stats.thread_count, 0);
        assert_eq!(stats.message_count, 0);
    }

    #[test]
    fn update_message_nonexistent_returns_error() {
        let (_temp, store) = make_store();
        store
            .ensure_thread(CreateConversationThread {
                id: "t1".to_string(),
                title: "Thread".to_string(),
                created_at: "2026-04-10T12:00:00Z".to_string(),
            })
            .unwrap();
        let result = store.update_message(
            "t1",
            "nonexistent",
            ConversationMessagePatch {
                extra_metadata: Some(json!({})),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn conversation_store_new() {
        let tmp = TempDir::new().unwrap();
        let store = ConversationStore::new(tmp.path().to_path_buf());
        let threads = store.list_threads().unwrap();
        assert!(threads.is_empty());
    }

    #[test]
    fn conversation_purge_stats_default() {
        let stats = ConversationPurgeStats::default();
        assert_eq!(stats.thread_count, 0);
        assert_eq!(stats.message_count, 0);
    }
}
