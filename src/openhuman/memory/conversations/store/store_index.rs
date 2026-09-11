//! Internal thread-folding and inverted-index helpers for
//! [`ConversationStore`]. Split out of `store.rs` to respect the repo's
//! 500-line-per-file limit. Every method here is `pub(super)` so the public
//! API in `store_ops.rs` (and the unit tests) can call it, but it stays out of
//! the crate's public surface.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::path::PathBuf;

use super::super::inverted_index::InvertedIndex;
use super::super::types::{ConversationMessage, ConversationThread};
use super::{
    append_jsonl, hex_encode, infer_labels, normalize_labels, read_jsonl, ConversationPurgeStats,
    ConversationStore, ThreadIndexEntry, ThreadLogEntry, CONVERSATION_INDEX_CACHE,
    THREADS_FILENAME, THREAD_MESSAGES_DIR,
};

impl ConversationStore {
    /// If no index entry exists for this workspace, serialize cold builders,
    /// start a short-lived append journal, snapshot the live thread IDs under
    /// the root metadata lock, release it, and read every JSONL file under its
    /// per-thread lock. Publication folds in every append journaled during the
    /// scan while holding metadata, so it cannot publish stale and never needs
    /// to retry under sustained write traffic.
    ///
    /// After this call returns, `with_index` will always find a warm entry and
    /// will not re-enter `populate_index_unlocked`.
    pub(super) fn prime_index_if_cold(&self) -> Result<(), String> {
        self.prime_index_if_cold_with_hook(|| {})
    }

    /// `after_scan` is a deterministic test seam for mutations that land
    /// after file reads but before publication. Production always passes a
    /// no-op closure through [`Self::prime_index_if_cold`].
    pub(super) fn prime_index_if_cold_with_hook(
        &self,
        mut after_scan: impl FnMut(),
    ) -> Result<(), String> {
        let key = self.root_dir();
        if CONVERSATION_INDEX_CACHE.lock().contains_key(&key) {
            return Ok(());
        }

        let _build = self.locks.index_build.lock();
        if CONVERSATION_INDEX_CACHE.lock().contains_key(&key) {
            return Ok(());
        }
        self.locks.begin_index_build();

        // This is header-only O(threads) work. Do not use
        // `list_threads_unlocked`: legacy workspaces can make that measure and
        // append stats for every thread while metadata is held.
        let thread_ids: Vec<String> = {
            let _metadata = self.locks.metadata.lock();
            match self.thread_index_unlocked() {
                Ok(index) => index.into_keys().collect(),
                Err(error) => {
                    self.locks.cancel_index_build();
                    return Err(error);
                }
            }
        };

        let mut idx = InvertedIndex::new();
        for thread_id in &thread_ids {
            let thread_lock = self.locks.thread(thread_id);
            let _thread = thread_lock.lock();
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
        after_scan();

        // Append finalization takes metadata too. Therefore every append is
        // either already in the journal drained here, or waits until after
        // publication and updates the now-warm cache directly.
        let _metadata = self.locks.metadata.lock();
        for (thread_id, message) in self.locks.finish_index_build() {
            idx.insert(&thread_id, message);
        }
        CONVERSATION_INDEX_CACHE.lock().insert(key, idx);
        Ok(())
    }

    /// Acquire an index that the caller has already warmed with
    /// [`Self::prime_index_if_cold`] and run `f` against it. The only
    /// production caller, `search_cross_thread_messages`, holds the root's
    /// lifecycle read guard across both calls, so purge cannot remove the
    /// entry between priming and access.
    pub(super) fn with_primed_index<R>(
        &self,
        f: impl FnOnce(&mut InvertedIndex) -> R,
    ) -> Result<R, String> {
        let key = self.root_dir();
        let mut cache = CONVERSATION_INDEX_CACHE.lock();
        let idx = cache
            .get_mut(&key)
            .ok_or_else(|| "conversation index missing after required prime".to_string())?;
        Ok(f(idx))
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
        self.root_dir.clone()
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

        Ok(Self::threads_from_index(index))
    }

    /// Fold and repair thread metadata without inverting the lock order used
    /// by message mutations. Recovery reads take the target thread lock before
    /// metadata; a newly-created thread discovered between passes is handled
    /// by the next iteration.
    pub(super) fn list_threads_coordinated(&self) -> Result<Vec<ConversationThread>, String> {
        let mut unreadable = HashSet::new();
        loop {
            let (index, missing) = {
                let _metadata = self.locks.metadata.lock();
                let index = self.thread_index_unlocked()?;
                let missing = index
                    .iter()
                    .filter(|(_, entry)| {
                        entry.message_count.is_none() || entry.last_message_at.is_none()
                    })
                    .filter(|(thread_id, _)| !unreadable.contains(*thread_id))
                    .map(|(thread_id, _)| thread_id.clone())
                    .collect::<Vec<_>>();
                (index, missing)
            };
            if missing.is_empty() {
                return Ok(Self::threads_from_index(index));
            }

            for thread_id in missing {
                let thread_lock = self.locks.thread(&thread_id);
                let _thread = thread_lock.lock();
                let _metadata = self.locks.metadata.lock();
                let index = self.thread_index_unlocked()?;
                let Some(entry) = index.get(&thread_id) else {
                    continue;
                };
                if entry.message_count.is_some() && entry.last_message_at.is_some() {
                    continue;
                }
                let Ok((count, last_message_at)) = self.measure_messages_unlocked(&thread_id)
                else {
                    // Quarantine this thread for this invocation so it neither
                    // blocks repairs for later threads nor causes the outer
                    // loop to retry it forever. A future list call retries it.
                    unreadable.insert(thread_id);
                    continue;
                };
                let resolved_last = last_message_at.unwrap_or_else(|| entry.created_at.clone());
                append_jsonl(
                    &self.ensure_root()?.join(THREADS_FILENAME),
                    &ThreadLogEntry::Stats {
                        thread_id,
                        message_count: count,
                        last_message_at: resolved_last,
                    },
                )?;
            }
        }
    }

    fn threads_from_index(index: BTreeMap<String, ThreadIndexEntry>) -> Vec<ConversationThread> {
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
        threads
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
