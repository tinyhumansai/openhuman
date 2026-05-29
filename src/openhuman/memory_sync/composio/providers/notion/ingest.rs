//! Notion → memory tree ingest plumbing.
//!
//! Owns the conversion from a single Notion page payload (post-extracted
//! by [`super::sync`]) into a [`DocumentInput`] and drives
//! [`memory::ingest_pipeline::ingest_document`] for that page.
//!
//! Mirrors the canonical Slack/Gmail per-source ingest layout
//! ([`super::super::slack::ingest`] / [`super::super::gmail::ingest`])
//! so retrieval surfaces (`memory.search`, `tree.read_chunk`,
//! `tree.browse`, the agent's recall path, summary trees) actually see
//! Notion content — pre-#2885 the provider wrote via
//! `MemoryClient::store_skill_sync` into the legacy `memory_docs` table,
//! invisible to the memory-tree retrieval stack.
//!
//! ## Source-id scope
//!
//! Source id is `notion:{connection_id}:{page_id}` — one source per
//! Notion page per connection. Page is the natural Notion grouping
//! ("one page = one document") so per-page ingest keeps the canonical
//! `SourceKind::Document` semantics and matches how the Gmail per-message
//! / Slack per-channel paths scope their sources.
//!
//! ## Re-ingest of edited pages
//!
//! Notion pages mutate (the cursor advances by `last_edited_time`).
//! Re-ingesting the same `(connection_id, page_id)` after the user edits
//! the page would short-circuit on the pipeline's `already_ingested`
//! gate — so the call site drops prior chunks for the same source_id
//! via `delete_chunks_by_source` *before* re-ingest, mirroring the
//! vault sync pattern in #2720. The provider's own
//! `SyncState::synced_ids` keyed by `{page_id}@{edited_time}` is the
//! authoritative "have we seen this revision?" check; this module only
//! runs when that says yes.
//!
//! ## Idempotency
//!
//! Chunk IDs are content-hashed inside the memory tree, so re-ingesting
//! a previously-seen page is an UPSERT on the same chunk row — no
//! duplicate chunks across syncs.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::{self, IngestResult};
use crate::openhuman::memory_store::chunks::store::{delete_chunks_by_source, is_source_ingested};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;

/// Platform identifier embedded in the canonical document body header.
/// Matches the value `memory_tree::retrieval::source::PLATFORM_KINDS`
/// expects for Notion-sourced documents.
pub const NOTION_PLATFORM: &str = "notion";

/// Tags attached to every Notion-ingested chunk. Stable list — retrieval
/// callers filter on these.
pub const DEFAULT_TAGS: &[&str] = &["notion", "ingested"];

/// Build the memory-tree source_id for one Notion page in one connection.
///
/// Stable across re-syncs of the same `(connection_id, page_id)` so the
/// pipeline's idempotency gate works correctly and the dedup-on-edit
/// path can map back to the prior chunks for cleanup before re-ingest.
pub(crate) fn notion_source_id(connection_id: &str, page_id: &str) -> String {
    format!("notion:{connection_id}:{page_id}")
}

/// Pretty-printed JSON body for one Notion page. We persist the *full*
/// Composio response payload (not just the title) so the chunked content
/// retains enough context for retrieval — Notion pages don't have a
/// natural single-string canonical body the way Slack messages do.
fn render_page_body(title: &str, page: &Value) -> String {
    let pretty = serde_json::to_string_pretty(page).unwrap_or_else(|_| "{}".to_string());
    format!("# {title}\n\n```json\n{pretty}\n```\n")
}

/// Parse a Notion `last_edited_time` (ISO 8601 / RFC 3339) into a
/// `DateTime<Utc>`, falling back to `Utc::now()` on failure so the
/// pipeline still gets a valid timestamp.
fn parse_edited_time(raw: Option<&str>) -> DateTime<Utc> {
    raw.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

/// Ingest one Notion page into the memory tree.
///
/// Caller (the provider's `sync` loop) is responsible for the
/// edit-detection / dedup state-machine (`SyncState::synced_ids` keyed
/// by `{page_id}@{edited_time}`) — this function trusts that the call
/// only happens for items the caller wants to admit.
///
/// On content updates of an already-ingested source_id, drops the prior
/// chunks first via `delete_chunks_by_source` so the pipeline's
/// `already_ingested` gate doesn't short-circuit the new content. This
/// mirrors the vault sync pattern in #2720.
///
/// Returns the number of chunks the pipeline wrote.
pub async fn ingest_page_into_memory_tree(
    config: &Config,
    connection_id: &str,
    page_id: &str,
    title: &str,
    edited_time: Option<&str>,
    page: &Value,
) -> Result<usize> {
    let source_id = notion_source_id(connection_id, page_id);

    // Re-sync of an edited page: drop prior chunks for the same source_id
    // before re-ingest. Both calls are sync rusqlite I/O so they share one
    // `spawn_blocking` hop.
    //
    // We gate `delete_chunks_by_source` behind `is_source_ingested` — the
    // delete path uses a `source_kind = ?1` scan with Rust-side
    // source-id filtering (see `store::delete_chunks_by_source_filter`),
    // so on a first-time ingest of a never-seen page it would scan every
    // Document-kind chunk just to find zero matches. `is_source_ingested`
    // is an indexed PK lookup against `mem_tree_ingested_sources`, so it
    // converts the common fresh-page case to one cheap `COUNT(*)` and only
    // pays the scan cost on actual re-ingests of edited pages.
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
            page_id = %page_id,
            removed_chunks = removed,
            "[composio:notion] ingest: re-ingest cleanup"
        );
    }

    let modified_at = parse_edited_time(edited_time);
    let body = render_page_body(title, page);
    let source_ref = Some(format!("notion://page/{page_id}"));

    let doc = DocumentInput {
        provider: NOTION_PLATFORM.to_string(),
        title: title.to_string(),
        body,
        modified_at,
        source_ref,
    };
    let tags: Vec<String> = DEFAULT_TAGS.iter().map(|s| s.to_string()).collect();
    let owner = format!("notion:{connection_id}");

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
                page_id = %page_id,
                chunks_written,
                already_ingested,
                "[composio:notion] ingest: page persisted"
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

    fn sample_page(page_id: &str, edited_time: &str) -> Value {
        json!({
            "id": page_id,
            "object": "page",
            "last_edited_time": edited_time,
            "properties": {
                "Name": { "title": [{ "plain_text": "Phoenix migration plan" }] }
            },
            "url": format!("https://www.notion.so/{}", page_id.replace('-', "")),
            "body_excerpt": "Phoenix ships Friday after staging review. Alice owns rollback, Bob on-call.",
        })
    }

    /// `notion_source_id` is stable across calls and namespaces
    /// `(connection_id, page_id)` distinctly. Pins the contract the
    /// re-ingest cleanup path relies on (`delete_chunks_by_source`
    /// against the same `source_id`).
    #[test]
    fn notion_source_id_is_stable_and_namespaced() {
        let a = notion_source_id("conn-1", "page-abc");
        let b = notion_source_id("conn-1", "page-abc");
        assert_eq!(a, b);
        assert_eq!(a, "notion:conn-1:page-abc");

        assert_ne!(
            notion_source_id("conn-1", "page-abc"),
            notion_source_id("conn-2", "page-abc"),
            "distinct connections must produce distinct source ids"
        );
        assert_ne!(
            notion_source_id("conn-1", "page-abc"),
            notion_source_id("conn-1", "page-xyz"),
            "distinct page ids must produce distinct source ids"
        );
    }

    /// `parse_edited_time` accepts valid ISO 8601 / RFC 3339 and falls
    /// back to `Utc::now()` on bad input rather than failing the ingest.
    /// We don't assert the now-fallback timestamp value (it's
    /// time-dependent) — just that we got a `DateTime<Utc>` back.
    #[test]
    fn parse_edited_time_handles_valid_and_invalid_inputs() {
        let good = parse_edited_time(Some("2026-05-28T12:34:56.000Z"));
        assert_eq!(good.format("%Y-%m-%d").to_string(), "2026-05-28");

        // Invalid / missing both fall through to `Utc::now()` — sanity
        // check that the result is "recent" (within last 5s).
        let bad = parse_edited_time(Some("not-a-timestamp"));
        assert!((Utc::now() - bad).num_seconds().abs() < 5);

        let missing = parse_edited_time(None);
        assert!((Utc::now() - missing).num_seconds().abs() < 5);
    }

    /// `render_page_body` produces a markdown document with the title
    /// header + the full page JSON pretty-printed in a fenced code
    /// block. Pins the chunked-content shape — without this the
    /// retrieval body becomes "just the title" and loses Notion-specific
    /// signal (properties, URL, excerpt) at search time.
    #[test]
    fn render_page_body_includes_title_header_and_pretty_json() {
        let page = json!({ "id": "p-1", "url": "https://notion.so/p1" });
        let body = render_page_body("Phoenix plan", &page);
        assert!(body.starts_with("# Phoenix plan\n"));
        assert!(body.contains("```json\n"));
        assert!(body.contains("\"id\": \"p-1\""));
        assert!(body.contains("\"url\": \"https://notion.so/p1\""));
    }

    /// The #2885 regression test.
    ///
    /// Before this migration, Notion sync routed through
    /// `MemoryClient::store_skill_sync` → `UnifiedMemory::upsert_document`
    /// → `memory_docs` (legacy backend). The memory-tree retrieval
    /// surfaces (which every modern caller reads from) saw zero rows.
    ///
    /// This test pins the new contract: a successful `ingest_page_into_memory_tree`
    /// call writes to `mem_tree_chunks` + `mem_tree_ingested_sources`,
    /// so the silent-failure mode can't reappear. Mirrors the
    /// `sync_writes_to_memory_tree` regression in `vault::sync` (#2720).
    #[tokio::test]
    async fn ingest_page_writes_to_memory_tree() {
        use crate::openhuman::memory_store::chunks::store::{count_chunks, is_source_ingested};

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-test";
        let page_id = "page-phoenix";
        let page = sample_page(page_id, "2026-05-28T10:00:00.000Z");

        let chunks_before = count_chunks(&cfg).expect("count_chunks before");

        let written = ingest_page_into_memory_tree(
            &cfg,
            connection_id,
            page_id,
            "Phoenix migration plan",
            Some("2026-05-28T10:00:00.000Z"),
            &page,
        )
        .await
        .expect("ingest_page_into_memory_tree");

        assert!(
            written > 0,
            "Notion ingest must write at least one chunk; got {written}"
        );

        // Core regression assertion: chunks landed in memory_tree.
        let chunks_after = count_chunks(&cfg).expect("count_chunks after");
        assert!(
            chunks_after > chunks_before,
            "ingest must populate mem_tree_chunks (#2885): {chunks_before} → {chunks_after}"
        );

        // Source registration.
        let cfg_for_blocking = cfg.clone();
        let expected = notion_source_id(connection_id, page_id);
        let registered = tokio::task::spawn_blocking(move || {
            is_source_ingested(&cfg_for_blocking, SourceKind::Document, &expected).unwrap_or(false)
        })
        .await
        .expect("source-check task join");
        assert!(
            registered,
            "source_id {} must be registered in mem_tree_ingested_sources",
            notion_source_id(connection_id, page_id)
        );
    }

    /// Re-ingesting an edited page (same `(connection_id, page_id)`,
    /// different content) cleans up prior chunks and writes fresh ones —
    /// the `delete_chunks_by_source` guard sidesteps the pipeline's
    /// `already_ingested` short-circuit that would otherwise drop the
    /// new revision.
    #[tokio::test]
    async fn re_ingesting_edited_page_replaces_prior_chunks() {
        use crate::openhuman::memory_store::chunks::store::count_chunks;

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-edit";
        let page_id = "page-edit";

        // First ingest.
        let v1 = sample_page(page_id, "2026-05-28T10:00:00.000Z");
        let first = ingest_page_into_memory_tree(
            &cfg,
            connection_id,
            page_id,
            "Phoenix plan v1",
            Some("2026-05-28T10:00:00.000Z"),
            &v1,
        )
        .await
        .expect("first ingest");
        assert!(first > 0);
        let after_first = count_chunks(&cfg).expect("count after first");

        // Re-ingest with different body — should NOT short-circuit, and
        // chunk count should not double (prior chunks dropped, new ones
        // written, net same per-page count for this body size).
        let v2 = json!({
            "id": page_id,
            "object": "page",
            "last_edited_time": "2026-05-29T10:00:00.000Z",
            "properties": { "Name": { "title": [{ "plain_text": "Phoenix plan revised" }] } },
            "body_excerpt": "Plan revised: ship Monday, Carol takes on-call instead.",
        });
        let second = ingest_page_into_memory_tree(
            &cfg,
            connection_id,
            page_id,
            "Phoenix plan v2",
            Some("2026-05-29T10:00:00.000Z"),
            &v2,
        )
        .await
        .expect("second ingest");
        assert!(
            second > 0,
            "edited page must actually re-ingest, not silently no-op"
        );
        let after_second = count_chunks(&cfg).expect("count after second");

        // The chunk count after the second ingest should equal the
        // count after the first (replaced one revision with another),
        // not double. Allow ±1 for any rounding in how the chunker
        // splits subtly-different markdown.
        assert!(
            after_second.abs_diff(after_first) <= 1,
            "edited page must replace prior chunks, not append: \
             after_first={after_first} after_second={after_second}"
        );
    }
}
