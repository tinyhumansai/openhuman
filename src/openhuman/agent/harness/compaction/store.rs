//! CCR — Compress-Cache-Retrieve store.
//!
//! When a compressor drops data (lossy paths), it stows the original here
//! keyed by a short content hash and emits a `retrieve_tool_output("<hash>")`
//! sentinel in the compacted text. The agent can call the
//! `retrieve_tool_output` tool to get the original back on demand — so even
//! aggressive compaction stays reversible and is safe under the always-on
//! default.
//!
//! Process-global and bounded: a fixed-capacity FIFO so a long session can't
//! grow it without bound. Keyed by content hash, so re-offloading identical
//! content is idempotent (the model sees a stable hash). Originals are not
//! persisted to disk — retrieval is best-effort within the session; an evicted
//! entry simply reports "no longer available", which is strictly better than
//! the pre-CCR behaviour (the data was gone the moment it was truncated).

use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// Max originals retained. ~256 large tool outputs is plenty for a session's
/// recent history while bounding worst-case memory.
const MAX_ENTRIES: usize = 256;
/// Bytes of the SHA-256 digest used for the key (→ 32 hex chars). Wide enough
/// that (a) collisions are infeasible and (b) the hash doubles as an
/// unguessable capability token — a session can only retrieve content whose
/// hash it was shown, so the process-global store can't be brute-force probed
/// across sessions. (Full per-session key namespacing is a tracked follow-up.)
const HASH_BYTES: usize = 16;

#[derive(Default)]
struct Inner {
    map: HashMap<String, String>,
    order: VecDeque<String>,
}

impl Inner {
    /// Insert `content` under `hash` (idempotent) and FIFO-evict down to
    /// [`MAX_ENTRIES`]. Pulled out of [`offload`] so the eviction policy can be
    /// unit-tested on a local instance without touching the process-global
    /// store (which would otherwise race other tests sharing it).
    fn insert(&mut self, hash: String, content: String) {
        if self.map.contains_key(&hash) {
            return;
        }
        self.map.insert(hash.clone(), content);
        self.order.push_back(hash);
        while self.order.len() > MAX_ENTRIES {
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }
    }
}

fn global() -> &'static Mutex<Inner> {
    static STORE: OnceLock<Mutex<Inner>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Inner::default()))
}

/// Stash `content` and return its short hash. Idempotent for identical content.
pub fn offload(content: &str) -> String {
    let hash = short_hash(content);
    global()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(hash.clone(), content.to_string());
    hash
}

/// Retrieve a previously-offloaded original by hash, if still cached.
pub fn retrieve(hash: &str) -> Option<String> {
    global()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .map
        .get(hash)
        .cloned()
}

/// Short hex content hash used as the CCR key.
pub fn short_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..HASH_BYTES])
}

#[cfg(test)]
mod tests {
    use super::*;

    // Round-trip tests use globally-unique content and collectively stay well
    // under MAX_ENTRIES, so they never evict each other even under parallel
    // execution. The eviction policy is exercised on a *local* `Inner` below so
    // it can't clobber the shared store other tests depend on.

    #[test]
    fn round_trips() {
        let original = "ccr round-trip unique payload ".repeat(50);
        let hash = offload(&original);
        assert_eq!(hash.len(), HASH_BYTES * 2);
        assert_eq!(retrieve(&hash).as_deref(), Some(original.as_str()));
    }

    #[test]
    fn idempotent_hash() {
        let a = offload("ccr idempotent unique payload content here");
        let b = offload("ccr idempotent unique payload content here");
        assert_eq!(a, b);
    }

    #[test]
    fn missing_hash_is_none() {
        // A 32-hex hash that no test content maps to.
        assert!(retrieve("ffffffffffffffffffffffffffffffff").is_none());
    }

    #[test]
    fn eviction_bounds_size() {
        // Exercise the FIFO eviction on a local instance — no shared state.
        let mut inner = Inner::default();
        for i in 0..(MAX_ENTRIES + 50) {
            inner.insert(format!("hash-{i}"), format!("content-{i}"));
        }
        assert!(inner.map.len() <= MAX_ENTRIES, "size bounded");
        assert!(!inner.map.contains_key("hash-0"), "oldest entry evicted");
        assert!(
            inner
                .map
                .contains_key(&format!("hash-{}", MAX_ENTRIES + 49)),
            "newest entry retained"
        );
    }
}
