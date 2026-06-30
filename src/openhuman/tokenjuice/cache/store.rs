//! CCR — Compress-Cache-Retrieve store.
//!
//! When a compressor drops data (lossy paths), the router stows the original
//! here keyed by a short content hash and embeds a retrieval marker in the
//! compacted text (see [`super::marker`]). The agent calls the
//! `tokenjuice_retrieve` tool to get the original back on demand — so even
//! aggressive compaction stays reversible and is safe under the always-on
//! default.
//!
//! Process-global and bounded by **both** an entry count and a total byte cap,
//! with an optional TTL. Keyed by content hash, so re-offloading identical
//! content is idempotent. An optional **disk tier** (configured from the core's
//! `workspace_dir`) persists originals across the session so retrieval can
//! survive memory eviction; the agent itself never writes there — only the core
//! does, through this module.

use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// Default max originals retained (entry-count cap).
pub const DEFAULT_MAX_ENTRIES: usize = 256;
/// Default total-bytes cap (64 MiB) so a few huge originals can't blow memory.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Bytes of the SHA-256 digest used for the key (→ 32 hex chars). Wide enough
/// that collisions are infeasible and the hash doubles as an unguessable
/// capability token.
const HASH_BYTES: usize = 16;

/// Tunable limits, settable once at startup from the `[tokenjuice]` config.
struct Limits {
    max_entries: usize,
    max_bytes: usize,
    ttl: Option<Duration>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_bytes: DEFAULT_MAX_BYTES,
            ttl: None,
        }
    }
}

fn limits() -> &'static RwLock<Limits> {
    static LIMITS: OnceLock<RwLock<Limits>> = OnceLock::new();
    LIMITS.get_or_init(|| RwLock::new(Limits::default()))
}

/// Optional on-disk tier root (under the core's workspace). `None` ⇒ in-memory only.
fn disk_root() -> &'static RwLock<Option<PathBuf>> {
    static ROOT: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    ROOT.get_or_init(|| RwLock::new(None))
}

/// Configure the cache limits (called once from config at startup).
pub fn configure(max_entries: usize, max_bytes: usize, ttl_secs: Option<u64>) {
    let mut l = limits().write().unwrap_or_else(|p| p.into_inner());
    l.max_entries = max_entries.max(1);
    l.max_bytes = max_bytes.max(1);
    l.ttl = ttl_secs.map(Duration::from_secs);
}

/// Enable the on-disk tier rooted at `root` (e.g. `<workspace>/.tokenjuice/ccr`).
/// Best-effort: directory creation failures disable the tier silently.
pub fn enable_disk_tier(root: PathBuf) {
    if std::fs::create_dir_all(&root).is_ok() {
        *disk_root().write().unwrap_or_else(|p| p.into_inner()) = Some(root);
    } else {
        log::warn!("[tokenjuice][ccr] could not create disk tier at {root:?}");
    }
}

/// Turn the on-disk tier off (e.g. when the setting is toggled off at runtime).
/// New offloads stop writing to disk; already-written files are left in place.
pub fn disable_disk_tier() {
    *disk_root().write().unwrap_or_else(|p| p.into_inner()) = None;
}

struct Entry {
    content: String,
    created: Instant,
}

#[derive(Default)]
struct Inner {
    map: HashMap<String, Entry>,
    order: VecDeque<String>,
    total_bytes: usize,
}

impl Inner {
    /// Insert `content` under `hash` (idempotent) and evict (FIFO) until both
    /// the entry-count and total-byte caps hold.
    ///
    /// Returns whether `hash` is still resident after eviction. A single
    /// original larger than `max_bytes` cannot be retained in memory under the
    /// byte cap — eviction would immediately drop the just-inserted entry — so
    /// `false` is returned and the caller must not advertise it as recoverable
    /// (the router declines lossy compaction or relies on the disk tier).
    fn insert(
        &mut self,
        hash: String,
        content: String,
        max_entries: usize,
        max_bytes: usize,
    ) -> bool {
        if let Some(entry) = self.map.get_mut(&hash) {
            entry.created = Instant::now();
            self.order.retain(|candidate| candidate != &hash);
            self.order.push_back(hash);
            return true;
        }
        let bytes = content.len();
        self.total_bytes += bytes;
        self.map.insert(
            hash.clone(),
            Entry {
                content,
                created: Instant::now(),
            },
        );
        self.order.push_back(hash.clone());
        while self.order.len() > max_entries || self.total_bytes > max_bytes {
            // Never evict the entry we just inserted to satisfy the cap when it
            // is the only thing keeping us over: that would make the original
            // unrecoverable the instant its footer is emitted. Stop and report
            // non-retention instead (one oversized item is rejected, not the
            // whole store wiped).
            if self.order.len() == 1 {
                break;
            }
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            if let Some(e) = self.map.remove(&evicted) {
                self.total_bytes = self.total_bytes.saturating_sub(e.content.len());
            }
        }
        // Retained iff still present (an oversized single entry over the byte
        // cap is dropped below) AND within the byte cap.
        if self.total_bytes > max_bytes {
            if let Some(e) = self.map.remove(&hash) {
                self.total_bytes = self.total_bytes.saturating_sub(e.content.len());
            }
            self.order.retain(|h| h != &hash);
            return false;
        }
        self.map.contains_key(&hash)
    }
}

fn global() -> &'static Mutex<Inner> {
    static STORE: OnceLock<Mutex<Inner>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Inner::default()))
}

/// Stash `content` and return its short hash. Idempotent for identical content.
pub fn offload(content: &str) -> String {
    offload_checked(content).0
}

/// Stash `content`, returning `(hash, retained)`. `retained` is `false` only
/// when the original could be kept neither in memory (it exceeds the byte cap)
/// nor on the disk tier — in which case the caller must NOT advertise it as
/// recoverable. Idempotent for identical content.
pub fn offload_checked(content: &str) -> (String, bool) {
    let hash = short_hash(content);
    let (max_entries, max_bytes) = {
        let l = limits().read().unwrap_or_else(|p| p.into_inner());
        (l.max_entries, l.max_bytes)
    };
    let mem_retained = global().lock().unwrap_or_else(|p| p.into_inner()).insert(
        hash.clone(),
        content.to_string(),
        max_entries,
        max_bytes,
    );

    // Mirror to the disk tier when enabled (best-effort). A successful disk
    // write keeps the original recoverable even when it was too big for memory.
    // Rewriting an existing hash intentionally refreshes the file mtime, which
    // is the TTL clock for disk-backed CCR entries.
    let mut disk_retained = false;
    if let Some(root) = disk_root()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    {
        let path = root.join(&hash);
        match std::fs::write(&path, content) {
            Ok(()) => disk_retained = true,
            Err(e) => log::debug!("[tokenjuice][ccr] disk write failed for {hash}: {e}"),
        }
    }
    (hash, mem_retained || disk_retained)
}

/// True if `hash` is a well-formed CCR token (exactly the generated hex digest).
/// Tokens come from agent-controlled tool args, and on a disk-tier miss they are
/// joined onto the CCR root — so anything other than the fixed hex shape (e.g.
/// `../../state/config.toml`) must be rejected before touching the filesystem to
/// prevent path traversal / arbitrary file reads through the recovery tool.
fn is_valid_token(hash: &str) -> bool {
    hash.len() == HASH_BYTES * 2 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Retrieve a previously-offloaded original by hash, if still available
/// (memory first, then the disk tier). Honours the TTL for both tiers.
pub fn retrieve(hash: &str) -> Option<String> {
    // Reject anything that isn't the generated token shape up front — guards the
    // disk-tier `root.join(hash)` below against path traversal.
    if !is_valid_token(hash) {
        return None;
    }
    let ttl = limits().read().unwrap_or_else(|p| p.into_inner()).ttl;
    {
        let mut inner = global().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = inner.map.get(hash) {
            if ttl.is_none_or(|t| entry.created.elapsed() < t) {
                return Some(entry.content.clone());
            }
            // Expired — drop it and fall through to disk.
            if let Some(e) = inner.map.remove(hash) {
                inner.total_bytes = inner.total_bytes.saturating_sub(e.content.len());
            }
        }
    }
    // Disk fallback.
    let root = disk_root()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()?;
    let path = root.join(hash);
    if disk_entry_expired(&path, ttl) {
        log::debug!("[tokenjuice][ccr] disk entry expired for {hash}");
        let _ = std::fs::remove_file(&path);
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn disk_entry_expired(path: &Path, ttl: Option<Duration>) -> bool {
    let Some(ttl) = ttl else {
        return false;
    };
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    modified.elapsed().is_ok_and(|elapsed| elapsed >= ttl)
}

/// The span/unit for a ranged retrieval.
#[derive(Debug, Clone, Copy)]
pub enum RangeUnit {
    Bytes,
    Lines,
}

/// Retrieve a slice of a previously-offloaded original. `start`/`end` are
/// 0-based, `end` exclusive; out-of-bounds ends are clamped. Returns `None`
/// only when the original isn't available at all.
pub fn retrieve_range(hash: &str, start: usize, end: usize, unit: RangeUnit) -> Option<String> {
    let original = retrieve(hash)?;
    if end <= start {
        return Some(String::new());
    }
    match unit {
        RangeUnit::Bytes => {
            // Clamp to char boundaries so we never split a UTF-8 sequence.
            let s = floor_char_boundary(&original, start.min(original.len()));
            let e = floor_char_boundary(&original, end.min(original.len()));
            Some(original[s..e].to_string())
        }
        RangeUnit::Lines => {
            let lines: Vec<&str> = original.lines().collect();
            let e = end.min(lines.len());
            if start >= lines.len() {
                return Some(String::new());
            }
            Some(lines[start..e].join("\n"))
        }
    }
}

/// Largest char boundary ≤ `idx` (std's `floor_char_boundary` is still nightly).
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Short hex content hash used as the CCR key/token.
pub fn short_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..HASH_BYTES])
}

/// Snapshot of cache occupancy for the debug controller / stats.
pub fn stats() -> (usize, usize) {
    let inner = global().lock().unwrap_or_else(|p| p.into_inner());
    (inner.map.len(), inner.total_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let original = "ccr round-trip unique payload alpha ".repeat(50);
        let hash = offload(&original);
        assert_eq!(hash.len(), HASH_BYTES * 2);
        assert_eq!(retrieve(&hash).as_deref(), Some(original.as_str()));
    }

    #[test]
    fn idempotent_hash() {
        let a = offload("ccr idempotent unique payload bravo content");
        let b = offload("ccr idempotent unique payload bravo content");
        assert_eq!(a, b);
    }

    #[test]
    fn missing_hash_is_none() {
        assert!(retrieve("ffffffffffffffffffffffffffffffff").is_none());
    }

    #[test]
    fn byte_cap_evicts_oldest() {
        let mut inner = Inner::default();
        // 10 entries of 100 bytes; cap at 500 bytes ⇒ keep ~5 newest.
        for i in 0..10 {
            inner.insert(format!("h{i}"), "x".repeat(100), 1000, 500);
        }
        assert!(
            inner.total_bytes <= 500,
            "byte cap held: {}",
            inner.total_bytes
        );
        assert!(!inner.map.contains_key("h0"), "oldest evicted");
        assert!(inner.map.contains_key("h9"), "newest retained");
    }

    #[test]
    fn oversized_single_entry_is_not_retained() {
        // One entry larger than the byte cap can't be kept; insert reports
        // non-retention so the router won't advertise it as recoverable.
        let mut inner = Inner::default();
        let retained = inner.insert("big".into(), "x".repeat(200), 100, 100);
        assert!(!retained, "oversized single entry must report not-retained");
        assert!(!inner.map.contains_key("big"));
        assert_eq!(inner.total_bytes, 0);
    }

    #[test]
    fn rejects_path_traversal_tokens() {
        // Non-hex / wrong-length tokens are rejected before any disk join.
        assert!(!is_valid_token("../../state/config.toml"));
        assert!(!is_valid_token("..%2f..%2fetc"));
        assert!(!is_valid_token("deadbeef")); // too short
        assert!(!is_valid_token(&"g".repeat(32))); // non-hex
        assert!(is_valid_token(&"a1b2c3d4".repeat(4))); // 32 hex chars
                                                        // retrieve() returns None for an invalid token regardless of cache state.
        assert!(retrieve("../../state/config.toml").is_none());
    }

    #[test]
    fn within_cap_entry_is_retained() {
        let mut inner = Inner::default();
        assert!(inner.insert("ok".into(), "x".repeat(50), 100, 100));
        assert!(inner.map.contains_key("ok"));
    }

    #[test]
    fn entry_cap_evicts_oldest() {
        let mut inner = Inner::default();
        for i in 0..60 {
            inner.insert(format!("e{i}"), format!("content-{i}"), 50, usize::MAX);
        }
        assert!(inner.map.len() <= 50);
        assert!(!inner.map.contains_key("e0"));
    }

    #[test]
    fn reoffloading_existing_entry_refreshes_ttl_and_order() {
        let mut inner = Inner::default();
        assert!(inner.insert("old".into(), "old payload".into(), 2, usize::MAX));
        assert!(inner.insert("fresh".into(), "fresh payload".into(), 2, usize::MAX));
        let stale_created = Instant::now() - Duration::from_secs(60);
        inner.map.get_mut("old").unwrap().created = stale_created;

        assert!(inner.insert("old".into(), "old payload".into(), 2, usize::MAX));
        assert!(
            inner.map["old"].created > stale_created,
            "existing entry timestamp should refresh"
        );
        assert_eq!(inner.order.back().map(String::as_str), Some("old"));

        assert!(inner.insert("third".into(), "third payload".into(), 2, usize::MAX));
        assert!(inner.map.contains_key("old"));
        assert!(!inner.map.contains_key("fresh"));
    }

    #[test]
    fn range_retrieval_lines_and_bytes() {
        let original = "line0\nline1\nline2\nline3\nline4";
        let hash = offload(original);
        assert_eq!(
            retrieve_range(&hash, 1, 3, RangeUnit::Lines).as_deref(),
            Some("line1\nline2")
        );
        assert_eq!(
            retrieve_range(&hash, 0, 5, RangeUnit::Bytes).as_deref(),
            Some("line0")
        );
        // Out-of-bounds end clamps.
        assert_eq!(
            retrieve_range(&hash, 4, 999, RangeUnit::Lines).as_deref(),
            Some("line4")
        );
    }

    #[test]
    fn disk_tier_survives_memory_miss() {
        let dir = std::env::temp_dir().join(format!("tj-ccr-{}", short_hash("disk-test-seed")));
        let _ = std::fs::remove_dir_all(&dir);
        enable_disk_tier(dir.clone());
        let original = "disk tier unique payload charlie ".repeat(40);
        let hash = offload(&original);
        // Simulate memory eviction by clearing the in-memory map directly.
        {
            let mut inner = global().lock().unwrap_or_else(|p| p.into_inner());
            inner.map.remove(&hash);
        }
        assert_eq!(
            retrieve(&hash).as_deref(),
            Some(original.as_str()),
            "disk fallback"
        );
        // Disable the tier for other tests and clean up.
        *disk_root().write().unwrap() = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_entry_ttl_uses_file_mtime() {
        let dir = std::env::temp_dir().join(format!("tj-ccr-ttl-{}", short_hash("disk-ttl")));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        std::fs::write(&path, "disk ttl payload").unwrap();

        assert!(!disk_entry_expired(&path, None));
        assert!(!disk_entry_expired(&path, Some(Duration::from_secs(60))));
        assert!(disk_entry_expired(&path, Some(Duration::ZERO)));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
