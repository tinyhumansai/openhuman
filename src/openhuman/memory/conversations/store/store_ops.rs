//! Public CRUD + search surface of [`ConversationStore`]. Split out of
//! `store.rs` to keep each source file under the repo's 500-line limit; this
//! is a descendant module of `store`, so it shares access to the private
//! statics, log-entry enum, and JSONL helpers defined there.

use std::fs;

use super::super::types::{
    is_deterministic_message_id, ConversationMessage, ConversationMessagePatch, ConversationThread,
    CreateConversationThread, CrossThreadHit,
};
use super::{
    append_jsonl, find_message_by_id, normalize_labels, read_jsonl, rewrite_jsonl,
    ConversationPurgeStats, ConversationStore, ThreadLogEntry, CONVERSATION_INDEX_CACHE,
    THREADS_FILENAME,
};

impl ConversationStore {
    /// Create or update a thread, appending an `Upsert` entry to `threads.jsonl`.
    pub fn ensure_thread(
        &self,
        request: CreateConversationThread,
    ) -> Result<ConversationThread, String> {
        let _lifecycle = self.locks.lifecycle.read();
        let thread_lock = self.locks.thread(&request.id);
        let _thread = thread_lock.lock();
        let _metadata = self.locks.metadata.lock();
        let root = self.ensure_root()?;
        let threads_path = root.join(THREADS_FILENAME);
        let now = request.created_at.clone();
        let labels = request.labels.clone().map(normalize_labels);
        append_jsonl(
            &threads_path,
            &ThreadLogEntry::Upsert {
                thread_id: request.id.clone(),
                title: request.title.clone(),
                created_at: request.created_at.clone(),
                updated_at: now,
                parent_thread_id: request.parent_thread_id.clone(),
                labels,
                personality_id: request.personality_id.clone(),
            },
        )?;
        self.thread_summary_unlocked(&request.id)?
            .ok_or_else(|| format!("thread {} missing after ensure", request.id))
    }

    /// List all live threads (folding the upsert/delete log).
    pub fn list_threads(&self) -> Result<Vec<ConversationThread>, String> {
        let _lifecycle = self.locks.lifecycle.read();
        self.list_threads_coordinated()
    }

    /// Read every persisted message for a thread in append order.
    pub fn get_messages(&self, thread_id: &str) -> Result<Vec<ConversationMessage>, String> {
        let _lifecycle = self.locks.lifecycle.read();
        let thread_lock = self.locks.thread(thread_id);
        let _thread = thread_lock.lock();
        {
            let _metadata = self.locks.metadata.lock();
            if !self.thread_exists_unlocked(thread_id)? {
                return Ok(Vec::new());
            }
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
    /// workspace dir per user — so this helper cannot cross that boundary.
    /// Issue #1505: the conversational durable-fact pipeline is async and
    /// batched, so cross-chat continuity needs a direct cross-thread reader to
    /// surface context the user shared in chat A when they ask a dependent
    /// question in chat B.
    ///
    /// Backed by an in-memory trigram/CJK-bigram inverted index
    /// (`super::super::inverted_index`). The legacy implementation walked every
    /// JSONL file and did `content.to_lowercase().contains(term)` per message,
    /// which is O(threads × messages × content_len). The index turns that into
    /// O(|posting lists|) for typical queries while preserving the previous
    /// scoring contract (`score = matched_terms / total_terms`, recency
    /// tiebreak).
    ///
    /// # Lock strategy (issue #2849)
    ///
    /// **Fast path (warm cache):** acquires the root lifecycle read guard and
    /// `CONVERSATION_INDEX_CACHE`, with no metadata or thread lock.
    ///
    /// **Cold path (first access):** snapshots the thread list under
    /// the root metadata lock (brief), then releases it before reading each
    /// JSONL file under its per-thread lock. This avoids blocking unrelated
    /// threads during the potentially-long rebuild. Appends completed during
    /// that scan are journaled and folded into the index atomically at
    /// publication.
    pub fn search_cross_thread_messages(
        &self,
        query: &str,
        limit: usize,
        exclude_thread_id: Option<&str>,
    ) -> Result<Vec<CrossThreadHit>, String> {
        // Warm the index without the metadata lock so concurrent
        // append_message / get_messages calls are not stalled during the
        // cold JSONL rebuild. After this returns the cache entry is
        // guaranteed to exist, so with_index will not trigger a second
        // rebuild.
        let _lifecycle = self.locks.lifecycle.read();
        self.prime_index_if_cold()?;
        self.with_primed_index(|idx| idx.search(query, limit, exclude_thread_id))
    }

    /// Append a message to the thread's JSONL file. Errors if the thread is missing.
    ///
    /// Persists via two separate fsync'd appends — the authoritative message
    /// row, then a compact `MessageAppended` stat entry. Thread reads reconcile
    /// that stat trail against the message file, repairing a crash between the
    /// two appends.
    ///
    /// Idempotent for the ids the core mints deterministically
    /// ([`is_deterministic_message_id`]): when the thread already holds a row
    /// with that id, nothing is written (no message row, no stat bump, no index
    /// insert) and the stored row is returned exactly as a fresh append would
    /// return its input. Two writers can legitimately persist the same reply —
    /// an autonomous run's `task_session::append_final` and the client that
    /// also persists the `chat_done` it announced (#5933) — and a thread must
    /// never carry two messages under one id (the frontend keys React and
    /// assistant-ui resources by it).
    ///
    /// The lookup is deliberately narrow. Every other id in the store is
    /// UUID-fresh by construction and cannot be re-presented, so it must not
    /// pay to have that verified: a lookup on *every* append would put a scan
    /// of the thread's transcript on every hot write and make growing a thread
    /// quadratic.
    pub fn append_message(
        &self,
        thread_id: &str,
        message: ConversationMessage,
    ) -> Result<ConversationMessage, String> {
        let _lifecycle = self.locks.lifecycle.read();
        let thread_lock = self.locks.thread(thread_id);
        let _thread = thread_lock.lock();
        {
            let _metadata = self.locks.metadata.lock();
            if !self.thread_exists_unlocked(thread_id)? {
                return Err(format!("thread {} not found", thread_id));
            }
        }
        let path = self.thread_messages_path(thread_id);
        if is_deterministic_message_id(&message.id) {
            if let Some(existing) = find_message_by_id(&path, &message.id)? {
                return Ok(existing);
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create conversation dir {}: {e}", parent.display()))?;
        }
        append_jsonl(&path, &message)?;
        // Bump the threads-log stat trail so subsequent `list_threads`
        // calls can compute (message_count, last_message_at) without
        // re-reading this file.
        {
            let _metadata = self.locks.metadata.lock();
            // The transcript row is already durable. Publish it to an active
            // cold-build journal and any warm cache before the derived stats
            // append, which may fail independently.
            self.locks.record_index_append(thread_id, &message);
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            if let Some(idx) = cache.get_mut(&self.root_dir()) {
                idx.insert(thread_id, message.clone());
            }
            drop(cache);
            let threads_path = self.root_dir().join(THREADS_FILENAME);
            append_jsonl(
                &threads_path,
                &ThreadLogEntry::MessageAppended {
                    thread_id: thread_id.to_string(),
                    last_message_at: message.created_at.clone(),
                },
            )?;
        }
        Ok(message)
    }

    /// Rewrite the thread title via a new `Upsert` log entry, preserving labels.
    pub fn update_thread_title(
        &self,
        thread_id: &str,
        title: &str,
        updated_at: &str,
    ) -> Result<ConversationThread, String> {
        let _lifecycle = self.locks.lifecycle.read();
        let thread_lock = self.locks.thread(thread_id);
        let _thread = thread_lock.lock();
        let _metadata = self.locks.metadata.lock();
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
        let _lifecycle = self.locks.lifecycle.read();
        let thread_lock = self.locks.thread(thread_id);
        let _thread = thread_lock.lock();
        let _metadata = self.locks.metadata.lock();
        let index = self.thread_index_unlocked()?;
        let entry = index
            .get(thread_id)
            .ok_or_else(|| format!("thread {} not found", thread_id))?;
        let threads_path = self.ensure_root()?.join(THREADS_FILENAME);
        let labels = normalize_labels(labels);
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
        let _lifecycle = self.locks.lifecycle.read();
        let thread_lock = self.locks.thread(thread_id);
        let _thread = thread_lock.lock();
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
        Ok(updated)
    }

    /// Append a `Delete` entry and remove the thread's messages file. Returns
    /// `false` if the thread did not exist.
    pub fn delete_thread(&self, thread_id: &str, deleted_at: &str) -> Result<bool, String> {
        // Deletion also evicts the thread's lock entry. Exclusive lifecycle
        // ownership prevents a new operation from retaining the old lock
        // while the registry entry is replaced.
        let _lifecycle = self.locks.lifecycle.write();
        let thread_lock = self.locks.thread(thread_id);
        let _thread = thread_lock.lock();
        {
            let _metadata = self.locks.metadata.lock();
            if !self.thread_exists_unlocked(thread_id)? {
                self.locks.remove_thread(thread_id);
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
        }
        let messages_path = self.thread_messages_path(thread_id);
        let remove_result = match fs::remove_file(&messages_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "delete conversation messages {}: {error}",
                messages_path.display()
            )),
        };
        // Evict on every path after the tombstone is durable, including a
        // filesystem deletion error. The lifecycle write guard prevents a new
        // operation from observing a replacement lock before this one drops.
        self.locks.remove_thread(thread_id);
        // Drop every indexed message for this thread so future searches
        // don't surface stale content.
        {
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            if let Some(idx) = cache.get_mut(&self.root_dir()) {
                idx.remove_thread(thread_id);
            }
        }
        remove_result?;
        Ok(true)
    }

    /// Wipe the entire conversation directory and re-create an empty layout.
    pub fn purge_threads(&self) -> Result<ConversationPurgeStats, String> {
        let _lifecycle = self.locks.lifecycle.write();
        let _metadata = self.locks.metadata.lock();
        let stats = self.purge_stats_unlocked()?;
        let root = self.root_dir();
        if root.exists() {
            fs::remove_dir_all(&root)
                .map_err(|e| format!("remove conversation dir {}: {e}", root.display()))?;
        }
        self.ensure_root()?;
        // Drop the cached inverted index — the workspace is now empty, and any
        // next search will lazily rebuild from the (now empty) JSONL tree.
        {
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            cache.remove(&root);
        }
        self.locks.clear_threads();
        Ok(stats)
    }
}
