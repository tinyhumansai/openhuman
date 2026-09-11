//! Lock registry for the JSONL conversation store.
//!
//! A root owns shared metadata (`threads.jsonl`) and many independent message
//! files. Keeping those synchronization scopes separate lets unrelated agent
//! sessions write their message files concurrently while preserving atomic
//! metadata appends and purge semantics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};

use parking_lot::{Mutex, RwLock};

use super::super::types::ConversationMessage;

#[derive(Debug, Default)]
pub(super) struct StoreLocks {
    /// Ordinary operations take a read guard; purge takes the write guard.
    pub(super) lifecycle: RwLock<()>,
    /// Serializes reads and appends of the root's shared `threads.jsonl`.
    pub(super) metadata: Mutex<()>,
    /// Only one cold scan may construct this root's in-memory index.
    pub(super) index_build: Mutex<()>,
    threads: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    /// Appends completed while a cold scan is in flight. `None` means no scan
    /// is active, so the warm-cache path alone owns index maintenance.
    pending_index_appends: Mutex<Option<Vec<(String, ConversationMessage)>>>,
}

impl StoreLocks {
    pub(super) fn thread(&self, thread_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.threads.lock();
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(thread_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(thread_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    pub(super) fn begin_index_build(&self) {
        let previous = self.pending_index_appends.lock().replace(Vec::new());
        debug_assert!(previous.is_none(), "index builds must be serialized");
    }

    pub(super) fn record_index_append(&self, thread_id: &str, message: &ConversationMessage) {
        if let Some(pending) = self.pending_index_appends.lock().as_mut() {
            pending.push((thread_id.to_string(), message.clone()));
        }
    }

    pub(super) fn finish_index_build(&self) -> Vec<(String, ConversationMessage)> {
        self.pending_index_appends.lock().take().unwrap_or_default()
    }

    pub(super) fn cancel_index_build(&self) {
        self.pending_index_appends.lock().take();
    }

    /// Call only while holding the lifecycle write guard.
    pub(super) fn remove_thread(&self, thread_id: &str) {
        self.threads.lock().remove(thread_id);
    }

    /// Call only while holding the lifecycle write guard.
    pub(super) fn clear_threads(&self) {
        self.threads.lock().clear();
    }

    #[cfg(test)]
    pub(super) fn thread_count(&self) -> usize {
        self.threads
            .lock()
            .values()
            .filter(|lock| lock.strong_count() > 0)
            .count()
    }
}

/// Separate `ConversationStore::new` calls for the same root must coordinate.
/// Weak entries avoid retaining one lock set for every temporary workspace a
/// long-running process has ever touched.
static ROOTS: LazyLock<Mutex<HashMap<PathBuf, Weak<StoreLocks>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn for_root(root: &Path) -> Arc<StoreLocks> {
    let root = normalized_root(root);
    let mut roots = ROOTS.lock();
    // A process may open many ephemeral workspaces over its lifetime. The
    // weak value avoids retaining each lock set; pruning dead values here also
    // prevents their path keys from making the registry itself grow forever.
    roots.retain(|_, locks| locks.strong_count() > 0);
    if let Some(existing) = roots.get(&root).and_then(Weak::upgrade) {
        return existing;
    }
    let locks = Arc::new(StoreLocks::default());
    roots.insert(root, Arc::downgrade(&locks));
    locks
}

/// Resolve aliases even before the conversation directory itself exists.
/// Canonicalizing the nearest existing ancestor handles symlinks and `..`;
/// the missing suffix is then appended without touching the filesystem.
pub(super) fn normalized_root(root: &Path) -> PathBuf {
    let absolute;
    let root = if root.is_absolute() {
        root
    } else {
        absolute = std::env::current_dir()
            .map(|cwd| cwd.join(root))
            .unwrap_or_else(|_| root.to_path_buf());
        &absolute
    };
    if let Ok(canonical) = root.canonicalize() {
        return canonical;
    }

    let mut suffix = Vec::new();
    let mut ancestor = root;
    loop {
        if let Ok(canonical) = ancestor.canonicalize() {
            return suffix
                .iter()
                .rev()
                .fold(canonical, |path, component| path.join(component));
        }
        let Some(name) = ancestor.file_name() else {
            return root.to_path_buf();
        };
        suffix.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return root.to_path_buf();
        };
        ancestor = parent;
    }
}
