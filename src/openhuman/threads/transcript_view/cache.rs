//! In-memory, mtime-keyed projection cache for the transcript view.
//!
//! The settled transcript is derived from `session_raw/*.jsonl` on every
//! request. Re-projecting a long thread on each page fetch is wasteful, so we
//! memoize per-thread projections keyed on the backing files' `(path, mtime,
//! len)` signature. An append (new turn, interrupted partial, sub-agent file)
//! changes the signature and transparently invalidates the entry — there are
//! **no disk writes** and no explicit invalidation call. The cache is bounded
//! (LRU over a few dozen threads) so a long-lived core can't grow it without
//! limit.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use super::project;
use super::types::ProjectedTranscript;

const LOG_PREFIX: &str = "[threads][transcript][cache]";

/// Number of distinct threads whose projections are retained. Beyond this the
/// least-recently-used entry is evicted.
const CACHE_CAPACITY: usize = 32;

/// Signature of one backing file — changes on any append/rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSig {
    path: PathBuf,
    mtime: Option<SystemTime>,
    len: u64,
}

fn file_sig(path: &Path) -> FileSig {
    let (mtime, len) = match std::fs::metadata(path) {
        Ok(meta) => (meta.modified().ok(), meta.len()),
        Err(_) => (None, 0),
    };
    FileSig {
        path: path.to_path_buf(),
        mtime,
        len,
    }
}

struct CacheEntry {
    signature: Vec<FileSig>,
    projected: Arc<ProjectedTranscript>,
}

#[derive(Default)]
struct CacheInner {
    entries: HashMap<String, CacheEntry>,
    /// LRU order — front is least-recently-used.
    order: VecDeque<String>,
}

/// Bounded per-thread projection cache.
#[derive(Default)]
pub struct TranscriptViewCache {
    inner: Mutex<CacheInner>,
}

impl TranscriptViewCache {
    /// Project `thread_id`'s transcript, serving a cached result when the
    /// backing files are unchanged. `None` when the thread has no transcript.
    pub fn get_or_project(
        &self,
        workspace_dir: &Path,
        thread_id: &str,
    ) -> Option<Arc<ProjectedTranscript>> {
        let (root_path, sub_paths) = project::resolve_files(workspace_dir, thread_id)?;
        let signature: Vec<FileSig> = std::iter::once(file_sig(&root_path))
            .chain(sub_paths.iter().map(|p| file_sig(p)))
            .collect();

        {
            let mut inner = self.inner.lock().ok()?;
            if let Some(entry) = inner.entries.get(thread_id) {
                if entry.signature == signature {
                    let projected = entry.projected.clone();
                    touch(&mut inner, thread_id);
                    log::debug!(
                        "{LOG_PREFIX} hit thread={thread_id} items={}",
                        projected.items.len()
                    );
                    return Some(projected);
                }
                log::debug!("{LOG_PREFIX} miss (signature changed) thread={thread_id}");
            } else {
                log::debug!("{LOG_PREFIX} miss (cold) thread={thread_id}");
            }
        }

        let projected = Arc::new(project::project_from_files(
            thread_id, &root_path, &sub_paths,
        ));

        let mut inner = self.inner.lock().ok()?;
        inner.entries.insert(
            thread_id.to_string(),
            CacheEntry {
                signature,
                projected: projected.clone(),
            },
        );
        touch(&mut inner, thread_id);
        evict_if_needed(&mut inner);
        log::debug!(
            "{LOG_PREFIX} stored thread={thread_id} items={} cache_size={}",
            projected.items.len(),
            inner.entries.len()
        );
        Some(projected)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().unwrap().entries.len()
    }
}

/// Move `thread_id` to the most-recently-used end of the LRU order.
fn touch(inner: &mut CacheInner, thread_id: &str) {
    if let Some(pos) = inner.order.iter().position(|t| t == thread_id) {
        inner.order.remove(pos);
    }
    inner.order.push_back(thread_id.to_string());
}

/// Evict least-recently-used entries until within [`CACHE_CAPACITY`].
fn evict_if_needed(inner: &mut CacheInner) {
    while inner.entries.len() > CACHE_CAPACITY {
        let Some(victim) = inner.order.pop_front() else {
            break;
        };
        inner.entries.remove(&victim);
        log::debug!("{LOG_PREFIX} evicted thread={victim}");
    }
}

/// Process-wide cache singleton. The transcript view is read-only and derived,
/// so one shared cache across RPC calls is correct.
pub fn global() -> &'static TranscriptViewCache {
    static CACHE: OnceLock<TranscriptViewCache> = OnceLock::new();
    CACHE.get_or_init(TranscriptViewCache::default)
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
