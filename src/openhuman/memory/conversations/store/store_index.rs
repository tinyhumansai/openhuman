//! Internal thread-folding and inverted-index helpers for
//! [`ConversationStore`]. Split out of `store.rs` to respect the repo's
//! 500-line-per-file limit. Every method here is `pub(super)` so the public
//! API in `store_ops.rs` (and the unit tests) can call it, but it stays out of
//! the crate's public surface.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::PathBuf;

use super::super::inverted_index::InvertedIndex;
use super::super::types::{ConversationMessage, ConversationThread};
use super::{
    append_jsonl, hex_encode, infer_labels, normalize_labels, read_jsonl, ConversationPurgeStats,
    ConversationStore, ThreadIndexEntry, ThreadLogEntry, CONVERSATION_INDEX_CACHE,
    CONVERSATION_STORE_LOCK, THREADS_FILENAME, THREAD_MESSAGES_DIR,
};

impl ConversationStore {
    /// If no index entry exists for this workspace, snapshot the live thread
    /// IDs under `CONVERSATION_STORE_LOCK` (fast — reads only `threads.jsonl`,
    /// no per-thread I/O), release that lock, read all per-thread JSONL files
    /// with no lock held (safe — append-only), then insert the built index
    /// into `CONVERSATION_INDEX_CACHE` using `entry().or_insert()` so a
    /// concurrent prime that finished first wins and ours is discarded.
    ///
    /// After this call returns, `with_index` will always find a warm entry and
    /// will not re-enter `populate_index_unlocked`.
    pub(super) fn prime_index_if_cold(&self) -> Result<(), String> {
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
        // Do NOT call `list_threads_unlocked` here.  For workspaces where any
        // thread has no `MessageAppended`/`Stats` history (common before the
        // Stats log was introduced), `list_threads_unlocked` triggers
        // `measure_messages_unlocked` + a `Stats` append per thread — all under
        // `CONVERSATION_STORE_LOCK` — reintroducing the multi-second stall this
        // function is designed to avoid.
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
        // append-only so reads are safe without synchronisation. A message
        // appended during this window stays absent from the in-memory index
        // until the next cold rebuild — the accepted tradeoff for issue #2849.
        let mut idx = InvertedIndex::new();
        for thread_id in &thread_ids {
            let path = self.thread_messages_path(thread_id);
            if !path.exists() {
                continue;
            }
            if let Ok(messages) = read_jsonl::<ConversationMessage>(&path) {
                for msg in messages {
                    idx.insert(thread_id, msg);
                }
            }
        }
        // Insert only if the key is still absent — a concurrent prime that
        // finished first wins; ours is discarded.
        {
            let mut cache = CONVERSATION_INDEX_CACHE.lock();
            cache.entry(key).or_insert(idx);
        }
        Ok(())
    }

    /// Acquire the cached inverted index for this workspace (building it from
    /// JSONL on first access) and run `f` against it. Caller MUST hold
    /// `CONVERSATION_STORE_LOCK` for the duration of the closure.
    ///
    /// In the normal path the index has already been warmed by
    /// `prime_index_if_cold`, so the cold-build branch here is a safety net for
    /// any future callers that bypass the priming step.
    pub(super) fn with_index<R>(
        &self,
        f: impl FnOnce(&mut InvertedIndex) -> R,
    ) -> Result<R, String> {
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
    /// case outside the outer lock. The JSONL files are the source of truth so
    /// a rebuild after a process crash is always safe.
    pub(super) fn populate_index_unlocked(&self, idx: &mut InvertedIndex) -> Result<(), String> {
        // Caller (`with_index`) already holds `CONVERSATION_STORE_LOCK`, so we
        // must NOT re-acquire it here — `parking_lot::Mutex` is not reentrant
        // and doing so would deadlock. Use the `_unlocked` thread reader
        // directly.
        let threads = self.list_threads_unlocked()?;
        for thread in threads {
            let path = self.thread_messages_path(&thread.id);
            if !path.exists() {
                continue;
            }
            let messages = match read_jsonl::<ConversationMessage>(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for msg in messages {
                idx.insert(&thread.id, msg);
            }
        }
        Ok(())
    }

    /// Ensure the `memory/conversations` directory tree (and an empty
    /// `threads.jsonl`) exists, returning the conversation root.
    pub(super) fn ensure_root(&self) -> Result<PathBuf, String> {
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

    /// Absolute path to this workspace's `memory/conversations` root.
    pub(super) fn root_dir(&self) -> PathBuf {
        self.workspace_dir.join("memory").join("conversations")
    }

    /// Absolute path to a thread's per-thread messages JSONL file. The thread
    /// id is hex-encoded so arbitrary ids map to filesystem-safe names.
    pub(super) fn thread_messages_path(&self, thread_id: &str) -> PathBuf {
        self.root_dir()
            .join(THREAD_MESSAGES_DIR)
            .join(format!("{}.jsonl", hex_encode(thread_id.as_bytes())))
    }

    pub(super) fn list_threads_unlocked(&self) -> Result<Vec<ConversationThread>, String> {
        let mut index = self.thread_index_unlocked()?;
        // Reconcile only cold/recovery entries whose derived stat trail is
        // absent. Once stats exist, routine list calls stay header-only rather
        // than rescanning every message file.
        let thread_ids = index
            .iter()
            .filter(|(_, entry)| entry.message_count.is_none() || entry.last_message_at.is_none())
            .map(|(thread_id, _)| thread_id.clone())
            .collect::<Vec<_>>();
        if !thread_ids.is_empty() {
            let threads_path = self.ensure_root()?.join(THREADS_FILENAME);
            for thread_id in &thread_ids {
                let Ok((count, last_message_at)) = self.measure_messages_unlocked(thread_id) else {
                    // One unreadable transcript must not make thread
                    // navigation unavailable. Leave it unreconciled so a
                    // later call can retry after repair.
                    continue;
                };
                // Treat created_at as last_message_at when there are no
                // messages — keeps the sort key meaningful and matches the
                // pre-refactor semantics.
                let resolved_last = last_message_at.unwrap_or_else(|| {
                    index
                        .get(thread_id)
                        .map(|e| e.created_at.clone())
                        .unwrap_or_default()
                });
                let differs = index.get(thread_id).is_none_or(|entry| {
                    entry.message_count != Some(count)
                        || entry.last_message_at.as_deref() != Some(resolved_last.as_str())
                });
                if differs {
                    append_jsonl(
                        &threads_path,
                        &ThreadLogEntry::Stats {
                            thread_id: thread_id.clone(),
                            message_count: count,
                            last_message_at: resolved_last.clone(),
                        },
                    )?;
                }
                if let Some(entry) = index.get_mut(thread_id) {
                    entry.message_count = Some(count);
                    entry.last_message_at = Some(resolved_last);
                }
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
                    labels: normalize_labels(entry.labels.clone()),
                    personality_id: entry.personality_id.clone(),
                }
            })
            .collect();
        threads.sort_by(|a, b| {
            timestamp_millis(&b.last_message_at)
                .cmp(&timestamp_millis(&a.last_message_at))
                .then_with(|| timestamp_millis(&b.created_at).cmp(&timestamp_millis(&a.created_at)))
        });
        Ok(threads)
    }

    /// Count messages and find the newest timestamp by reading the per-thread
    /// JSONL file. This is the authoritative source used to reconcile the
    /// compact thread stat trail after either side of a two-file append crash.
    pub(super) fn measure_messages_unlocked(
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

    pub(super) fn thread_summary_unlocked(
        &self,
        thread_id: &str,
    ) -> Result<Option<ConversationThread>, String> {
        let index = self.thread_index_unlocked()?;
        let entry = match index.get(thread_id) {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let (message_count, last_message_at) = match (entry.message_count, &entry.last_message_at) {
            (Some(count), Some(last)) => (count, last.clone()),
            _ => match self.measure_messages_unlocked(thread_id) {
                Ok((count, last)) => (count, last.unwrap_or_else(|| entry.created_at.clone())),
                Err(_) => (
                    entry.message_count.unwrap_or(0),
                    entry
                        .last_message_at
                        .clone()
                        .unwrap_or_else(|| entry.created_at.clone()),
                ),
            },
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
            labels: normalize_labels(entry.labels.clone()),
            personality_id: entry.personality_id.clone(),
        }))
    }

    pub(super) fn thread_exists_unlocked(&self, thread_id: &str) -> Result<bool, String> {
        Ok(self.thread_index_unlocked()?.contains_key(thread_id))
    }

    /// Fold `threads.jsonl` into the current per-thread state. Header-only:
    /// reads no per-thread message files.
    pub(super) fn thread_index_unlocked(
        &self,
    ) -> Result<BTreeMap<String, ThreadIndexEntry>, String> {
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
                            labels
                                .map(normalize_labels)
                                .unwrap_or_else(|| existing.labels.clone()),
                            existing.message_count,
                            existing.last_message_at.clone(),
                            personality_id.or_else(|| existing.personality_id.clone()),
                        ),
                        None => {
                            let inferred = labels
                                .map(normalize_labels)
                                .unwrap_or_else(|| infer_labels(&thread_id));
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
                        // backfill path in `list_threads_unlocked` can do the
                        // one-shot file read instead of producing a wrong "1"
                        // here.
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

    pub(super) fn purge_stats_unlocked(&self) -> Result<ConversationPurgeStats, String> {
        let threads = self.list_threads_unlocked()?;
        let message_count = threads.iter().map(|thread| thread.message_count).sum();
        Ok(ConversationPurgeStats {
            thread_count: threads.len(),
            message_count,
        })
    }
}

fn timestamp_millis(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .unwrap_or(i64::MIN)
}
