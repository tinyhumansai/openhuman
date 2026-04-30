//! Eager prefetch of the cross-source memory-tree digest into the
//! orchestrator's session context (Phase 4 follow-on, #710 wiring).
//!
//! The orchestrator answers "what happened this week?" / "what's been going
//! on with X?" style questions out of the user's own ingested memory. We
//! pre-load a 7-day global digest on the session's first turn AND
//! periodically thereafter (every [`REFRESH_INTERVAL`]) so long-running
//! conversations stay current with newly-ingested memory without needing
//! the LLM to round-trip a tool call. The injection rides on the user
//! message (NOT the system prompt) to keep the KV-cache prefix stable.
//!
//! When the workspace has no global summaries yet (early-life workspaces
//! or no ingest configured), [`TreeContextLoader::load`] returns an empty
//! string and the caller silently no-ops. The session-side timestamp is
//! still bumped on those empty results so an empty workspace doesn't get
//! re-queried every turn.
//!
//! Failure is non-fatal by design — the orchestrator must still be able to
//! reply when the memory tree is unavailable, mis-configured, or empty. We
//! log the failure mode and return `Ok(String::new())` so the caller can
//! concatenate without branching.

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::retrieval::query_global;

/// Default lookback window for the eager digest. Mirrors the language in
/// the orchestrator prompt ("7-day digest pre-loaded into session context").
pub const DEFAULT_WINDOW_DAYS: u32 = 7;

/// Minimum wall-clock interval between successive prefetches in the same
/// session. The first turn always fetches (timestamp is `None`); subsequent
/// turns re-prefetch only after this interval has elapsed since the last
/// successful call. Picked to balance freshness in long-running chats
/// against repeating the same digest content when no new ingest has
/// happened — the typical case for short bursts of conversation.
pub const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Per-hit content cap to keep the injection bounded; long summary bodies
/// would otherwise dominate the prompt budget.
const MAX_CONTENT_CHARS: usize = 500;

/// Number of hits to surface from the digest. The recap typically returns
/// one hit per fold (day/week/month) — three is enough headroom for a
/// 7-day window without flooding the system prompt.
const MAX_HITS: usize = 3;

const HEADER: &str = "[Memory tree — last 7 days]\n";

/// Decide whether the per-session prefetch should run on the current turn.
/// Pure: no I/O, no clock — `now` is supplied so callers (and tests) stay
/// deterministic. Returns `true` when no prefetch has happened yet
/// (`last == None`) or when at least `interval` has elapsed since the last.
pub fn should_prefetch(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
    interval: std::time::Duration,
) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t) >= interval,
    }
}

pub struct TreeContextLoader;

impl TreeContextLoader {
    /// Build the eager-prefetch context block for the current workspace.
    ///
    /// Returns:
    /// - `Ok("")` when the workspace has no global digest yet, or when
    ///   `query_global` returns an error (logged at warn level).
    /// - `Ok(rendered)` with the formatted block when there are hits.
    pub async fn load(config: &Config) -> anyhow::Result<String> {
        log::debug!(
            "[memory_tree] tree_loader.load window_days={}",
            DEFAULT_WINDOW_DAYS
        );
        let resp = match query_global(config, DEFAULT_WINDOW_DAYS).await {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "[memory_tree] tree_loader.load: query_global failed — returning empty: {e}"
                );
                return Ok(String::new());
            }
        };
        if resp.hits.is_empty() {
            log::debug!("[memory_tree] tree_loader.load: no hits — empty context");
            return Ok(String::new());
        }

        let mut out = String::with_capacity(HEADER.len() + MAX_HITS * MAX_CONTENT_CHARS);
        out.push_str(HEADER);
        for hit in resp.hits.iter().take(MAX_HITS) {
            let snippet = if hit.content.chars().count() > MAX_CONTENT_CHARS {
                crate::openhuman::util::truncate_with_ellipsis(&hit.content, MAX_CONTENT_CHARS)
            } else {
                hit.content.clone()
            };
            out.push_str(&format!(
                "- [{}] {}\n",
                hit.tree_kind.as_str(),
                snippet.replace('\n', " ")
            ));
        }
        out.push('\n');
        log::debug!(
            "[memory_tree] tree_loader.load returning chars={}",
            out.chars().count()
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn empty_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        };
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    #[tokio::test]
    async fn load_returns_empty_when_no_global_digest() {
        let (_tmp, cfg) = empty_config();
        let s = TreeContextLoader::load(&cfg).await.unwrap();
        assert!(
            s.is_empty(),
            "fresh workspace has no global digest — expected empty string, got: {s}"
        );
    }

    #[test]
    fn should_prefetch_when_never_fetched() {
        let now = std::time::Instant::now();
        assert!(should_prefetch(None, now, REFRESH_INTERVAL));
    }

    #[test]
    fn should_not_prefetch_within_interval() {
        let now = std::time::Instant::now();
        let one_minute_ago = now - std::time::Duration::from_secs(60);
        assert!(!should_prefetch(
            Some(one_minute_ago),
            now,
            REFRESH_INTERVAL
        ));
    }

    #[test]
    fn should_prefetch_after_interval_elapsed() {
        let now = std::time::Instant::now();
        let thirty_one_min_ago = now - std::time::Duration::from_secs(31 * 60);
        assert!(should_prefetch(
            Some(thirty_one_min_ago),
            now,
            REFRESH_INTERVAL
        ));
    }

    #[test]
    fn should_prefetch_at_exact_interval_boundary() {
        let now = std::time::Instant::now();
        let exactly_thirty_min_ago = now - REFRESH_INTERVAL;
        assert!(should_prefetch(
            Some(exactly_thirty_min_ago),
            now,
            REFRESH_INTERVAL
        ));
    }
}
