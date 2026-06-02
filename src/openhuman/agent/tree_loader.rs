//! Eager prefetch of recent memory-tree activity into the orchestrator's
//! session context (Phase 4 follow-on, #710 wiring).
//!
//! NOTE (#3170): this loader is **not currently wired into the agent turn
//! loop**. The unconditional 7-day digest injection was removed because it
//! duplicated the on-demand memory-tree retrieval tools (the smart
//! multi-strategy walk, #3077) and — unlike those tools — ignored the
//! memory-tree on/off toggle (which only gates the ingestion scheduler, not
//! this read path). The module is retained so an opt-in eager digest can be
//! re-wired behind a proper read-side gate later. Its public surface is
//! still exercised by `tests/inference_agent_raw_coverage_e2e.rs`. The
//! historical behavior is described below (in past tense) for that future
//! re-wiring.
//!
//! Historically, the orchestrator answered "what happened this week?" /
//! "what's been going on with X?" style questions out of the user's own
//! ingested memory. It pre-loaded a 7-day recap on the session's first turn
//! AND periodically thereafter (every [`REFRESH_INTERVAL`]) so long-running
//! conversations stayed current with newly-ingested memory without needing
//! the LLM to round-trip a tool call. The injection rode on the user message
//! (NOT the system prompt) to keep the KV-cache prefix stable.
//!
//! The recap was assembled by walking the **per-source** trees across the
//! window (the global digest tree was removed — source trees plus the
//! entity index are the substrate). When the workspace had no source
//! summaries yet (early-life workspaces or no ingest configured),
//! [`TreeContextLoader::load`] returned an empty string and the caller
//! silently no-op'd. The session-side timestamp was bumped on those empty
//! results too so an empty workspace didn't get re-queried every turn.
//!
//! Failure was non-fatal by design — the orchestrator had to stay able to
//! reply when the memory tree was unavailable, mis-configured, or empty. The
//! loader logs the failure mode and returns `Ok(String::new())` so a caller
//! can concatenate without branching.

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::retrieval::query_source;

/// Default lookback window for the eager digest. Retained for a future
/// re-wiring (see the module-level NOTE) — not actively consumed now that
/// the prefetch is unwired. Mirrored the language in the orchestrator
/// prompt ("7-day digest pre-loaded into session context").
pub const DEFAULT_WINDOW_DAYS: u32 = 7;

/// Minimum wall-clock interval between successive prefetches in the same
/// session. Retained for a future re-wiring (see the module-level NOTE) —
/// no caller drives this cadence today. Historically: the first turn always
/// fetched (timestamp `None`); later turns re-prefetched only after this
/// interval elapsed since the last successful call — picked to balance
/// freshness in long-running chats against repeating the same digest when no
/// new ingest had happened.
pub const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Per-hit content cap to keep the injection bounded; long summary bodies
/// would otherwise dominate the prompt budget.
const MAX_CONTENT_CHARS: usize = 500;

/// Number of hits to surface from the recap. Source-tree summaries across
/// the window are newest-first — three is enough headroom for a 7-day
/// window without flooding the system prompt.
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
    /// NOTE (#3170): not called from the agent turn loop anymore — see the
    /// module-level NOTE. Retained for tests and a possible future re-wiring.
    ///
    /// Returns:
    /// - `Ok("")` when the workspace has no source summaries yet, or when
    ///   `query_source` returns an error (logged at warn level).
    /// - `Ok(rendered)` with the formatted block when there are hits.
    pub async fn load(config: &Config) -> anyhow::Result<String> {
        log::debug!(
            "[memory_tree] tree_loader.load window_days={}",
            DEFAULT_WINDOW_DAYS
        );
        // Walk all source trees across the window (no source filter, no
        // semantic query — just the most recent summaries everywhere).
        let resp = match query_source(
            config,
            None,
            None,
            Some(DEFAULT_WINDOW_DAYS),
            None,
            MAX_HITS,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "[memory_tree] tree_loader.load: query_source failed — returning empty: {e}"
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
    async fn load_returns_empty_when_no_source_summaries() {
        let (_tmp, cfg) = empty_config();
        let s = TreeContextLoader::load(&cfg).await.unwrap();
        assert!(
            s.is_empty(),
            "fresh workspace has no source summaries — expected empty string, got: {s}"
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
