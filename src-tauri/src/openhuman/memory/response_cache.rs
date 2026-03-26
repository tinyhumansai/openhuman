//! Response cache — avoid burning tokens on repeated prompts.
//!
//! Stores LLM responses in a separate SQLite table keyed by a SHA-256 hash of
//! `(model, system_prompt_hash, user_prompt)`. Entries expire after a
//! configurable TTL (default: 1 hour). The cache is optional and disabled by
//! default — users opt in via `[memory] response_cache_enabled = true`.

use anyhow::Result;
use chrono::{Duration, Local};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Response cache backed by a dedicated SQLite database.
///
/// Lives alongside `brain.db` as `response_cache.db` so it can be
/// independently wiped without touching memories.
pub struct ResponseCache {
    conn: Mutex<Connection>,
    #[allow(dead_code)]
    db_path: PathBuf,
    ttl_minutes: i64,
    max_entries: usize,
}

impl ResponseCache {
    /// Open (or create) the response cache database.
    pub fn new(workspace_dir: &Path, ttl_minutes: u32, max_entries: usize) -> Result<Self> {
        let db_dir = workspace_dir.join("memory");
        std::fs::create_dir_all(&db_dir)?;
        let db_path = db_dir.join("response_cache.db");

        let conn = Connection::open(&db_path)?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA temp_store   = MEMORY;",
        )?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS response_cache (
                prompt_hash TEXT PRIMARY KEY,
                model       TEXT NOT NULL,
                response    TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL,
                accessed_at TEXT NOT NULL,
                hit_count   INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_rc_accessed ON response_cache(accessed_at);
            CREATE INDEX IF NOT EXISTS idx_rc_created ON response_cache(created_at);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
            ttl_minutes: i64::from(ttl_minutes),
            max_entries,
        })
    }

    /// Build a deterministic cache key from model + system prompt + user prompt.
    pub fn cache_key(model: &str, system_prompt: Option<&str>, user_prompt: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(model.as_bytes());
        hasher.update(b"|");
        if let Some(sys) = system_prompt {
            hasher.update(sys.as_bytes());
        }
        hasher.update(b"|");
        hasher.update(user_prompt.as_bytes());
        let hash = hasher.finalize();
        format!("{:064x}", hash)
    }

    /// Look up a cached response. Returns `None` on miss or expired entry.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock();

        let now = Local::now();
        let cutoff = (now - Duration::minutes(self.ttl_minutes)).to_rfc3339();

        let mut stmt = conn.prepare(
            "SELECT response FROM response_cache
             WHERE prompt_hash = ?1 AND created_at > ?2",
        )?;

        let result: Option<String> = stmt.query_row(params![key, cutoff], |row| row.get(0)).ok();

        if result.is_some() {
            // Bump hit count and accessed_at
            let now_str = now.to_rfc3339();
            conn.execute(
                "UPDATE response_cache
                 SET accessed_at = ?1, hit_count = hit_count + 1
                 WHERE prompt_hash = ?2",
                params![now_str, key],
            )?;
        }

        Ok(result)
    }

    /// Store a response in the cache.
    pub fn put(&self, key: &str, model: &str, response: &str, token_count: u32) -> Result<()> {
        let conn = self.conn.lock();

        let now = Local::now().to_rfc3339();

        conn.execute(
            "INSERT OR REPLACE INTO response_cache
             (prompt_hash, model, response, token_count, created_at, accessed_at, hit_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![key, model, response, token_count, now, now],
        )?;

        // Evict expired entries
        let cutoff = (Local::now() - Duration::minutes(self.ttl_minutes)).to_rfc3339();
        conn.execute(
            "DELETE FROM response_cache WHERE created_at <= ?1",
            params![cutoff],
        )?;

        // LRU eviction if over max_entries
        #[allow(clippy::cast_possible_wrap)]
        let max = self.max_entries as i64;
        conn.execute(
            "DELETE FROM response_cache WHERE prompt_hash IN (
                SELECT prompt_hash FROM response_cache
                ORDER BY accessed_at ASC
                LIMIT MAX(0, (SELECT COUNT(*) FROM response_cache) - ?1)
            )",
            params![max],
        )?;

        Ok(())
    }

    /// Return cache statistics: (total_entries, total_hits, total_tokens_saved).
    pub fn stats(&self) -> Result<(usize, u64, u64)> {
        let conn = self.conn.lock();

        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM response_cache", [], |row| row.get(0))?;

        let hits: i64 = conn.query_row(
            "SELECT COALESCE(SUM(hit_count), 0) FROM response_cache",
            [],
            |row| row.get(0),
        )?;

        let tokens_saved: i64 = conn.query_row(
            "SELECT COALESCE(SUM(token_count * hit_count), 0) FROM response_cache",
            [],
            |row| row.get(0),
        )?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok((count as usize, hits as u64, tokens_saved as u64))
    }

    /// Wipe the entire cache from the web UI.
    pub fn clear(&self) -> Result<usize> {
        let conn = self.conn.lock();

        let affected = conn.execute("DELETE FROM response_cache", [])?;
        Ok(affected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_cache(ttl_minutes: u32) -> (TempDir, ResponseCache) {
        let tmp = TempDir::new().unwrap();
        let cache = ResponseCache::new(tmp.path(), ttl_minutes, 1000).unwrap();
        (tmp, cache)
    }

    #[test]
    fn cache_key_deterministic() {
        let k1 = ResponseCache::cache_key("gpt-4", Some("sys"), "hello");
        let k2 = ResponseCache::cache_key("gpt-4", Some("sys"), "hello");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn cache_key_varies_by_model() {
        let k1 = ResponseCache::cache_key("gpt-4", None, "hello");
        let k2 = ResponseCache::cache_key("claude-3", None, "hello");
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_varies_by_system_prompt() {
        let k1 = ResponseCache::cache_key("gpt-4", Some("You are helpful"), "hello");
        let k2 = ResponseCache::cache_key("gpt-4", Some("You are rude"), "hello");
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_key_varies_by_prompt() {
        let k1 = ResponseCache::cache_key("gpt-4", None, "hello");
        let k2 = ResponseCache::cache_key("gpt-4", None, "goodbye");
        assert_ne!(k1, k2);
    }

    #[test]
    fn put_and_get() {
        let (_tmp, cache) = temp_cache(60);
        let key = ResponseCache::cache_key("gpt-4", None, "What is Rust?");

        cache
            .put(&key, "gpt-4", "Rust is a systems programming language.", 25)
            .unwrap();

        let result = cache.get(&key).unwrap();
        assert_eq!(
            result.as_deref(),
            Some("Rust is a systems programming language.")
        );
    }

    #[test]
    fn miss_returns_none() {
        let (_tmp, cache) = temp_cache(60);
        let result = cache.get("nonexistent_key").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn expired_entry_returns_none() {
        let (_tmp, cache) = temp_cache(0); // 0-minute TTL → everything is instantly expired
        let key = ResponseCache::cache_key("gpt-4", None, "test");

        cache.put(&key, "gpt-4", "response", 10).unwrap();

        // The entry was created with created_at = now(), but TTL is 0 minutes,
        // so cutoff = now() - 0 = now(). The entry's created_at is NOT > cutoff.
        let result = cache.get(&key).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn hit_count_incremented() {
        let (_tmp, cache) = temp_cache(60);
        let key = ResponseCache::cache_key("gpt-4", None, "hello");

        cache.put(&key, "gpt-4", "Hi!", 5).unwrap();

        // 3 hits
        for _ in 0..3 {
            let _ = cache.get(&key).unwrap();
        }

        let (_, total_hits, _) = cache.stats().unwrap();
        assert_eq!(total_hits, 3);
    }

    #[test]
    fn tokens_saved_calculated() {
        let (_tmp, cache) = temp_cache(60);
        let key = ResponseCache::cache_key("gpt-4", None, "explain rust");

        cache.put(&key, "gpt-4", "Rust is...", 100).unwrap();

        // 5 cache hits × 100 tokens = 500 tokens saved
        for _ in 0..5 {
            let _ = cache.get(&key).unwrap();
        }

        let (_, _, tokens_saved) = cache.stats().unwrap();
        assert_eq!(tokens_saved, 500);
    }

    #[test]
    fn lru_eviction() {
        let tmp = TempDir::new().unwrap();
        let cache = ResponseCache::new(tmp.path(), 60, 3).unwrap(); // max 3 entries

        for i in 0..5 {
            let key = ResponseCache::cache_key("gpt-4", None, &format!("prompt {i}"));
            cache
                .put(&key, "gpt-4", &format!("response {i}"), 10)
                .unwrap();
        }

        let (count, _, _) = cache.stats().unwrap();
        assert!(count <= 3, "Should have at most 3 entries after eviction");
    }

    #[test]
    fn clear_wipes_all() {
        let (_tmp, cache) = temp_cache(60);

        for i in 0..10 {
            let key = ResponseCache::cache_key("gpt-4", None, &format!("prompt {i}"));
            cache
                .put(&key, "gpt-4", &format!("response {i}"), 10)
                .unwrap();
        }

        let cleared = cache.clear().unwrap();
        assert_eq!(cleared, 10);

        let (count, _, _) = cache.stats().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn stats_empty_cache() {
        let (_tmp, cache) = temp_cache(60);
        let (count, hits, tokens) = cache.stats().unwrap();
        assert_eq!(count, 0);
        assert_eq!(hits, 0);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn overwrite_same_key() {
        let (_tmp, cache) = temp_cache(60);
        let key = ResponseCache::cache_key("gpt-4", None, "question");

        cache.put(&key, "gpt-4", "answer v1", 20).unwrap();
        cache.put(&key, "gpt-4", "answer v2", 25).unwrap();

        let result = cache.get(&key).unwrap();
        assert_eq!(result.as_deref(), Some("answer v2"));

        let (count, _, _) = cache.stats().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn unicode_prompt_handling() {
        let (_tmp, cache) = temp_cache(60);
        let key = ResponseCache::cache_key("gpt-4", None, "日本語のテスト 🦀");

        cache
            .put(&key, "gpt-4", "はい、Rustは素晴らしい", 30)
            .unwrap();

        let result = cache.get(&key).unwrap();
        assert_eq!(result.as_deref(), Some("はい、Rustは素晴らしい"));
    }

    // ── §4.4 Cache eviction under pressure tests ─────────────

    #[test]
    fn lru_eviction_keeps_most_recent() {
        let tmp = TempDir::new().unwrap();
        let cache = ResponseCache::new(tmp.path(), 60, 3).unwrap();

        // Insert 3 entries
        for i in 0..3 {
            let key = ResponseCache::cache_key("gpt-4", None, &format!("prompt {i}"));
            cache
                .put(&key, "gpt-4", &format!("response {i}"), 10)
                .unwrap();
        }

        // Access entry 0 to make it recently used
        let key0 = ResponseCache::cache_key("gpt-4", None, "prompt 0");
        let _ = cache.get(&key0).unwrap();

        // Insert entry 3 (triggers eviction)
        let key3 = ResponseCache::cache_key("gpt-4", None, "prompt 3");
        cache.put(&key3, "gpt-4", "response 3", 10).unwrap();

        let (count, _, _) = cache.stats().unwrap();
        assert!(count <= 3, "cache must not exceed max_entries");

        // Entry 0 was recently accessed and should survive
        let entry0 = cache.get(&key0).unwrap();
        assert!(
            entry0.is_some(),
            "recently accessed entry should survive LRU eviction"
        );
    }

    #[test]
    fn cache_handles_zero_max_entries() {
        let tmp = TempDir::new().unwrap();
        let cache = ResponseCache::new(tmp.path(), 60, 0).unwrap();

        let key = ResponseCache::cache_key("gpt-4", None, "test");
        // Should not panic even with max_entries=0
        cache.put(&key, "gpt-4", "response", 10).unwrap();

        let (count, _, _) = cache.stats().unwrap();
        assert_eq!(count, 0, "cache with max_entries=0 should evict everything");
    }

    #[test]
    fn cache_concurrent_reads_no_panic() {
        let tmp = TempDir::new().unwrap();
        let cache = std::sync::Arc::new(ResponseCache::new(tmp.path(), 60, 100).unwrap());

        let key = ResponseCache::cache_key("gpt-4", None, "concurrent");
        cache.put(&key, "gpt-4", "response", 10).unwrap();

        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache = std::sync::Arc::clone(&cache);
            let key = key.clone();
            handles.push(std::thread::spawn(move || {
                let _ = cache.get(&key).unwrap();
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let (_, hits, _) = cache.stats().unwrap();
        assert_eq!(hits, 10, "all concurrent reads should register as hits");
    }
}
