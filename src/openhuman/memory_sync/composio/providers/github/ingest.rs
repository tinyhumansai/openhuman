//! GitHub → memory tree ingest plumbing.
//!
//! Owns the conversion from a single GitHub issue/PR payload (post-extracted
//! by [`super::sync`]) into a [`DocumentInput`] and drives
//! [`memory::ingest_pipeline::ingest_document`] for that item.
//!
//! Mirrors the canonical Slack/Gmail/Notion per-source ingest layout
//! ([`super::super::slack::ingest`] / [`super::super::gmail::ingest`] /
//! [`super::super::notion::ingest`]) so retrieval surfaces
//! (`memory.search`, `tree.read_chunk`, `tree.browse`, the agent's
//! recall path, summary trees) actually see GitHub content — pre-#2885
//! the provider wrote via `MemoryClient::store_skill_sync` into the
//! legacy `memory_docs` table, invisible to the memory-tree retrieval
//! stack.
//!
//! ## Source-id scope
//!
//! Source id is `github:{connection_id}:{issue_id}` — one source per
//! GitHub issue or pull request per connection. Issue and PR are the
//! natural GitHub grouping ("one issue/PR = one document") so per-item
//! ingest keeps the canonical `SourceKind::Document` semantics and
//! matches how the Gmail per-message / Slack per-channel paths scope
//! their sources.
//!
//! ## Re-ingest of edited issues
//!
//! GitHub issues and PRs mutate (the cursor advances by `updated_at` —
//! title, body, labels, assignees, and comments all bump it). Re-ingesting
//! the same `(connection_id, issue_id)` after an update would
//! short-circuit on the pipeline's `already_ingested` gate — so the call
//! site drops prior chunks for the same source_id via
//! `delete_chunks_by_source` *before* re-ingest, mirroring the vault
//! sync pattern in #2720 and the Notion provider in #2887. The
//! provider's own `SyncState::synced_ids` keyed by
//! `{issue_id}@{updated_at}` is the authoritative "have we seen this
//! revision?" check; this module only runs when that says yes.
//!
//! ## Idempotency
//!
//! Chunk IDs are content-hashed inside the memory tree, so re-ingesting
//! a previously-seen issue is an UPSERT on the same chunk row — no
//! duplicate chunks across syncs.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::{self, IngestResult};
use crate::openhuman::memory_store::chunks::store::{delete_chunks_by_source, is_source_ingested};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;

/// Platform identifier embedded in the canonical document body header.
/// Matches the value `memory_tree::retrieval::source::PLATFORM_KINDS`
/// expects for GitHub-sourced documents.
pub const GITHUB_PLATFORM: &str = "github";

/// Tags attached to every GitHub-ingested chunk. Stable list — retrieval
/// callers filter on these.
pub const DEFAULT_TAGS: &[&str] = &["github", "ingested"];

/// Build the memory-tree source_id for one GitHub issue/PR in one connection.
///
/// Stable across re-syncs of the same `(connection_id, issue_id)` so the
/// pipeline's idempotency gate works correctly and the dedup-on-edit
/// path can map back to the prior chunks for cleanup before re-ingest.
///
/// `issue_id` is whatever stable identifier `super::sync::extract_issue_id`
/// produces — typically GitHub's numeric internal ID, falling back to
/// `owner/repo#number` parsed from `html_url`. Either form is stable
/// per-issue, so callers don't need to normalise before constructing
/// the source id.
pub(crate) fn github_source_id(connection_id: &str, issue_id: &str) -> String {
    format!("github:{connection_id}:{issue_id}")
}

/// Pretty-printed JSON body for one GitHub issue/PR. We persist the *full*
/// Composio response payload (not just the title + body markdown) so the
/// chunked content retains enough context for retrieval — labels,
/// assignees, milestone, comment counts, and state transitions are all
/// meaningful retrieval signal that a "just the body" projection would
/// drop.
fn render_issue_body(title: &str, issue: &Value) -> String {
    let pretty = serde_json::to_string_pretty(issue).unwrap_or_else(|_| "{}".to_string());
    format!("# {title}\n\n```json\n{pretty}\n```\n")
}

/// Parse a GitHub `updated_at` timestamp (ISO 8601 / RFC 3339, e.g.
/// `"2024-05-21T15:30:00Z"`) into a `DateTime<Utc>`, falling back to
/// `Utc::now()` on failure so the pipeline still gets a valid timestamp.
fn parse_updated_at(raw: Option<&str>) -> DateTime<Utc> {
    raw.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

/// Best-effort extraction of the issue's `html_url` for `source_ref`.
///
/// Using the canonical GitHub URL (rather than a custom `github://` scheme
/// like Notion does) keeps the audit trail one-click navigable — clicking
/// `source_ref` in any downstream UI lands on the real issue page.
fn extract_html_url(issue: &Value) -> Option<String> {
    issue
        .get("html_url")
        .and_then(|v| v.as_str())
        .or_else(|| issue.pointer("/data/html_url").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

/// Ingest one GitHub issue (or PR — same shape) into the memory tree.
///
/// Caller (the provider's `sync` loop) is responsible for the
/// edit-detection / dedup state-machine (`SyncState::synced_ids` keyed
/// by `{issue_id}@{updated_at}`) — this function trusts that the call
/// only happens for items the caller wants to admit.
///
/// On content updates of an already-ingested source_id, drops the prior
/// chunks first via `delete_chunks_by_source` so the pipeline's
/// `already_ingested` gate doesn't short-circuit the new content. This
/// mirrors the vault sync pattern in #2720 and the Notion provider in
/// #2887.
///
/// Returns the number of chunks the pipeline wrote.
pub async fn ingest_issue_into_memory_tree(
    config: &Config,
    connection_id: &str,
    issue_id: &str,
    title: &str,
    updated_at: Option<&str>,
    issue: &Value,
) -> Result<usize> {
    let source_id = github_source_id(connection_id, issue_id);

    // Re-sync of an edited issue: drop prior chunks for the same source_id
    // before re-ingest. Both calls are sync rusqlite I/O so they share one
    // `spawn_blocking` hop.
    //
    // We gate `delete_chunks_by_source` behind `is_source_ingested` — the
    // delete path uses a `source_kind = ?1` scan with Rust-side
    // source-id filtering (see `store::delete_chunks_by_source_filter`),
    // so on a first-time ingest of a never-seen issue it would scan every
    // Document-kind chunk just to find zero matches. `is_source_ingested`
    // is an indexed PK lookup against `mem_tree_ingested_sources`, so it
    // converts the common fresh-issue case to one cheap `COUNT(*)` and
    // only pays the scan cost on actual re-ingests of edited items.
    let cfg_for_blocking = config.clone();
    let source_for_blocking = source_id.clone();
    let removed = tokio::task::spawn_blocking(move || -> Result<usize> {
        if is_source_ingested(
            &cfg_for_blocking,
            SourceKind::Document,
            &source_for_blocking,
        )? {
            delete_chunks_by_source(
                &cfg_for_blocking,
                SourceKind::Document,
                &source_for_blocking,
            )
        } else {
            Ok(0)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("delete-prior task join error: {e}"))??;
    if removed > 0 {
        tracing::debug!(
            connection_id = %connection_id,
            issue_id = %issue_id,
            removed_chunks = removed,
            "[composio:github] ingest: re-ingest cleanup"
        );
    }

    let modified_at = parse_updated_at(updated_at);
    let body = render_issue_body(title, issue);
    let source_ref = extract_html_url(issue);

    let doc = DocumentInput {
        provider: GITHUB_PLATFORM.to_string(),
        title: title.to_string(),
        body,
        modified_at,
        source_ref,
    };
    let tags: Vec<String> = DEFAULT_TAGS.iter().map(|s| s.to_string()).collect();
    let owner = format!("github:{connection_id}");

    match ingest_pipeline::ingest_document(config, &source_id, &owner, tags, doc).await {
        Ok(IngestResult {
            chunks_written,
            already_ingested,
            ..
        }) => {
            // The delete-first guard above prevents `already_ingested` on
            // the normal update path. Seeing it here means the prior
            // chunks were already absent (fresh ingest into a primed
            // memory_tree) — fine, just log at debug.
            tracing::debug!(
                connection_id = %connection_id,
                issue_id = %issue_id,
                chunks_written,
                already_ingested,
                "[composio:github] ingest: issue persisted"
            );
            Ok(chunks_written)
        }
        Err(err) => Err(anyhow::anyhow!(
            // `{err:#}` (alternate formatter) bakes in the anyhow context
            // chain so provider.rs's `tracing::warn!(error = %e)` doesn't
            // strip the underlying cause (DB / embedding / persist failure)
            // when it Displays the wrapped error.
            "ingest_document failed for {source_id}: {err:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().expect("tempdir");
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Disable strict embedding so the pipeline accepts chunks without
        // a live embedder (matches the
        // `memory::sync_pipeline_e2e_test::test_config` shape).
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn sample_issue(issue_id: u64, updated_at: &str, title: &str, body: &str) -> Value {
        json!({
            "id": issue_id,
            "number": 99,
            "title": title,
            "body": body,
            "state": "open",
            "html_url": format!("https://github.com/acme/core/issues/{}", issue_id),
            "user": { "login": "octocat", "id": 1u64 },
            "labels": [
                { "name": "bug", "color": "d73a4a" },
                { "name": "memory", "color": "0e8a16" }
            ],
            "updated_at": updated_at,
            "created_at": "2026-05-01T00:00:00Z",
            "comments": 3,
            "assignees": [{ "login": "alice" }],
        })
    }

    /// `github_source_id` is stable across calls and namespaces
    /// `(connection_id, issue_id)` distinctly. Pins the contract the
    /// re-ingest cleanup path relies on (`delete_chunks_by_source`
    /// against the same `source_id`).
    #[test]
    fn github_source_id_is_stable_and_namespaced() {
        let a = github_source_id("conn-1", "12345");
        let b = github_source_id("conn-1", "12345");
        assert_eq!(a, b);
        assert_eq!(a, "github:conn-1:12345");

        // owner/repo#number slug form also stable.
        assert_eq!(
            github_source_id("conn-1", "acme/core#42"),
            "github:conn-1:acme/core#42"
        );

        assert_ne!(
            github_source_id("conn-1", "12345"),
            github_source_id("conn-2", "12345"),
            "distinct connections must produce distinct source ids"
        );
        assert_ne!(
            github_source_id("conn-1", "12345"),
            github_source_id("conn-1", "67890"),
            "distinct issue ids must produce distinct source ids"
        );
    }

    /// `parse_updated_at` accepts valid ISO 8601 / RFC 3339 and falls
    /// back to `Utc::now()` on bad input rather than failing the ingest.
    /// We don't assert the now-fallback timestamp value (it's
    /// time-dependent) — just that we got a `DateTime<Utc>` back.
    #[test]
    fn parse_updated_at_handles_valid_and_invalid_inputs() {
        let good = parse_updated_at(Some("2026-05-28T12:34:56Z"));
        assert_eq!(good.format("%Y-%m-%d").to_string(), "2026-05-28");

        // Invalid / missing both fall through to `Utc::now()` — sanity
        // check that the result is "recent" (within last 5s).
        let bad = parse_updated_at(Some("not-a-timestamp"));
        assert!((Utc::now() - bad).num_seconds().abs() < 5);

        let missing = parse_updated_at(None);
        assert!((Utc::now() - missing).num_seconds().abs() < 5);
    }

    /// `render_issue_body` produces a markdown document with the title
    /// header + the full issue JSON pretty-printed in a fenced code
    /// block. Pins the chunked-content shape — without this the
    /// retrieval body becomes "just the title" and loses GitHub-specific
    /// signal (labels, assignees, comment count, state) at search time.
    #[test]
    fn render_issue_body_includes_title_header_and_pretty_json() {
        let issue = sample_issue(123, "2026-05-28T10:00:00Z", "Fix the bug", "Repro steps:");
        let body = render_issue_body("GitHub: acme/core#99: Fix the bug", &issue);
        assert!(body.starts_with("# GitHub: acme/core#99: Fix the bug\n"));
        assert!(body.contains("```json\n"));
        assert!(body.contains("\"title\": \"Fix the bug\""));
        assert!(body.contains("\"bug\""));
        assert!(body.contains("\"assignees\""));
    }

    /// `extract_html_url` prefers the top-level field and falls back to
    /// the `data.html_url` wrapper Composio sometimes returns.
    #[test]
    fn extract_html_url_handles_both_envelope_shapes() {
        let direct = json!({ "html_url": "https://github.com/x/y/issues/1" });
        assert_eq!(
            extract_html_url(&direct),
            Some("https://github.com/x/y/issues/1".to_string())
        );

        let wrapped = json!({ "data": { "html_url": "https://github.com/x/y/issues/2" } });
        assert_eq!(
            extract_html_url(&wrapped),
            Some("https://github.com/x/y/issues/2".to_string())
        );

        let missing = json!({ "id": 1u64 });
        assert!(extract_html_url(&missing).is_none());
    }

    /// The #2885 regression test.
    ///
    /// Before this migration, GitHub sync routed through
    /// `MemoryClient::store_skill_sync` → `UnifiedMemory::upsert_document`
    /// → `memory_docs` (legacy backend). The memory-tree retrieval
    /// surfaces (which every modern caller reads from) saw zero rows.
    ///
    /// This test pins the new contract: a successful
    /// `ingest_issue_into_memory_tree` call writes to `mem_tree_chunks`
    /// + `mem_tree_ingested_sources`, so the silent-failure mode can't
    /// reappear. Mirrors the `sync_writes_to_memory_tree` regression
    /// in `vault::sync` (#2720) and the equivalent in
    /// `composio::providers::notion::ingest` (#2887).
    #[tokio::test]
    async fn ingest_issue_writes_to_memory_tree() {
        use crate::openhuman::memory_store::chunks::store::{count_chunks, is_source_ingested};

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-test";
        let issue_id = "987654321";
        let issue = sample_issue(987654321, "2026-05-28T10:00:00Z", "Fix flaky test", "Repro");

        let chunks_before = count_chunks(&cfg).expect("count_chunks before");

        let written = ingest_issue_into_memory_tree(
            &cfg,
            connection_id,
            issue_id,
            "GitHub: acme/core#99: Fix flaky test",
            Some("2026-05-28T10:00:00Z"),
            &issue,
        )
        .await
        .expect("ingest_issue_into_memory_tree");

        assert!(
            written > 0,
            "GitHub ingest must write at least one chunk; got {written}"
        );

        // Core regression assertion: chunks landed in memory_tree.
        let chunks_after = count_chunks(&cfg).expect("count_chunks after");
        assert!(
            chunks_after > chunks_before,
            "ingest must populate mem_tree_chunks (#2885): {chunks_before} → {chunks_after}"
        );

        // Source registration.
        let cfg_for_blocking = cfg.clone();
        let expected = github_source_id(connection_id, issue_id);
        let registered = tokio::task::spawn_blocking(move || {
            is_source_ingested(&cfg_for_blocking, SourceKind::Document, &expected).unwrap_or(false)
        })
        .await
        .expect("source-check task join");
        assert!(
            registered,
            "source_id {} must be registered in mem_tree_ingested_sources",
            github_source_id(connection_id, issue_id)
        );
    }

    /// Re-ingesting an edited issue (same `(connection_id, issue_id)`,
    /// different content) cleans up prior chunks and writes fresh ones —
    /// the `delete_chunks_by_source` guard sidesteps the pipeline's
    /// `already_ingested` short-circuit that would otherwise drop the
    /// new revision.
    #[tokio::test]
    async fn re_ingesting_edited_issue_replaces_prior_chunks() {
        use crate::openhuman::memory_store::chunks::store::count_chunks;

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-edit";
        let issue_id = "555";

        // First ingest.
        let v1 = sample_issue(
            555,
            "2026-05-28T10:00:00Z",
            "Flaky test on Linux",
            "Original repro: bisect points at #1234.",
        );
        let first = ingest_issue_into_memory_tree(
            &cfg,
            connection_id,
            issue_id,
            "GitHub: acme/core#99: Flaky test on Linux",
            Some("2026-05-28T10:00:00Z"),
            &v1,
        )
        .await
        .expect("first ingest");
        assert!(first > 0);
        let after_first = count_chunks(&cfg).expect("count after first");

        // Re-ingest with different body (issue was edited with new
        // description and labels) — should NOT short-circuit, and chunk
        // count should not double (prior chunks dropped, new ones
        // written, net same per-issue count for this body size).
        let v2 = json!({
            "id": 555u64,
            "number": 99,
            "title": "Flaky test on Linux — root cause found",
            "body": "Root cause: race in scheduler. PR #1300 fixes.",
            "state": "open",
            "html_url": "https://github.com/acme/core/issues/99",
            "labels": [
                { "name": "bug" },
                { "name": "needs-review" }
            ],
            "updated_at": "2026-05-29T11:22:33Z",
        });
        let second = ingest_issue_into_memory_tree(
            &cfg,
            connection_id,
            issue_id,
            "GitHub: acme/core#99: Flaky test on Linux — root cause found",
            Some("2026-05-29T11:22:33Z"),
            &v2,
        )
        .await
        .expect("second ingest");
        assert!(
            second > 0,
            "edited issue must actually re-ingest, not silently no-op"
        );
        let after_second = count_chunks(&cfg).expect("count after second");

        // The chunk count after the second ingest should equal the
        // count after the first (replaced one revision with another),
        // not double. Allow ±1 for any rounding in how the chunker
        // splits subtly-different markdown.
        assert!(
            after_second.abs_diff(after_first) <= 1,
            "edited issue must replace prior chunks, not append: \
             after_first={after_first} after_second={after_second}"
        );
    }
}
