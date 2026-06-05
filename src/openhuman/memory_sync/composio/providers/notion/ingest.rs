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
//! Source id is `notion:{connection_id}:{page_id}` — one document identity
//! per Notion page per connection. The source tree / raw archive scope is
//! `notion:{connection_id}`, so all pages selected by one Notion connection
//! accumulate under one source folder and one source tree instead of creating
//! one graph source per page.
//!
//! ## Page body content
//!
//! `NOTION_FETCH_DATA` (the sync's listing call) returns page metadata +
//! `properties` only — never the page body. The provider fetches each
//! new/edited page's rendered body via `NOTION_GET_PAGE_MARKDOWN` and passes
//! it here as `markdown_body`; [`render_page_body`] then emits the body as the
//! primary content with the metadata JSON appended. Database/task rows have no
//! body blocks (empty markdown) → metadata-only, which is correct since their
//! data lives in `properties`.
//!
//! ## Re-ingest of edited pages (non-destructive, versioned)
//!
//! Notion pages mutate (`last_edited_time` advances). An edit is ingested
//! **non-destructively**: `last_edited_time` becomes the document `version`,
//! and the source-ingest gate is keyed by `{source_id}@{version}` (via
//! [`ingest_pipeline::ingest_document_versioned`]) so a new revision is
//! admitted *alongside* the prior one — the old chunks are NOT deleted
//! (replacing the pre-versioning `delete_chunks_by_source` path). A
//! [`JobKind::SealDocument`] job then builds this revision's per-document
//! subtree (one versioned doc-root) and merges it into the connection tree;
//! retrieval surfaces only the latest version per document. The provider's
//! `SyncState::synced_ids` keyed by `{page_id}@{edited_time}` is the
//! authoritative "have we seen this revision?" check — unchanged pages are
//! skipped before this module ever runs.
//!
//! ## Idempotency
//!
//! Chunk IDs are content-hashed, so re-ingesting identical content is an
//! UPSERT on the same chunk row — no duplicate chunks across syncs.

use std::sync::OnceLock;

use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline;
use crate::openhuman::memory_queue::store::enqueue;
use crate::openhuman::memory_queue::types::{NewJob, SealDocumentPayload};
use crate::openhuman::memory_queue::wake_workers;
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

/// Build the source tree / raw archive scope for one Notion connection.
///
/// Keep this deliberately item-free. Page ids belong in [`notion_source_id`]
/// for document dedupe, not in the user-visible source graph.
pub(crate) fn notion_source_scope(connection_id: &str) -> String {
    format!("notion:{connection_id}")
}

/// Strip the noise `NOTION_GET_PAGE_MARKDOWN` bakes into the rendered body
/// before it reaches [`render_page_body`] / the ingest pipeline.
///
/// The Composio markdown export is functional but token-expensive: image
/// links carry S3 *signed* query strings (`?X-Amz-Algorithm=…&X-Amz-Signature=…`
/// — hundreds of tokens of pure noise that also rotate every fetch, defeating
/// content-hash idempotency), inline formatting is wrapped in HTML `<span>`
/// tags that smuggle `discussion-urls=`/`color=` attributes, user @-mentions
/// render as `<mention-user .../>` tags, and hard line breaks come through as
/// `<br>`. None of that is useful retrieval signal. This collapses it to clean
/// markdown so the chunked content is the actual prose, headings, and lists.
///
/// Regexes are compiled once and cached in process-global `OnceLock`s.
/// Conservative by design — it only touches the specific noise patterns above
/// and leaves ordinary markdown (headings, lists, links, fenced code) intact.
fn clean_notion_markdown(md: &str) -> String {
    // `![alt](https://host/file.png?X-Amz-...&...)` → `![alt](https://host/file.png)`.
    // Match an image link whose URL has a query string, dropping everything
    // from the `?` up to the closing paren. The URL itself can't contain `)`
    // or whitespace, so `[^)\s?]*` for the path and `[^)]*` for the query is safe.
    static IMG_QUERY: OnceLock<Regex> = OnceLock::new();
    // `<span ...>inner</span>` → `inner`. Strips the open tag (with any
    // attributes: `color=`, `discussion-urls=`, …) and the matching close tag,
    // keeping the inner text. Non-greedy attribute match so adjacent spans
    // don't get swallowed.
    static SPAN_OPEN: OnceLock<Regex> = OnceLock::new();
    static SPAN_CLOSE: OnceLock<Regex> = OnceLock::new();
    // `<mention-user .../>` and `<mention-user ...></mention-user>` → removed.
    static MENTION: OnceLock<Regex> = OnceLock::new();
    // `<br>` / `<br/>` / `<br />` → newline.
    static BR: OnceLock<Regex> = OnceLock::new();
    // 3+ consecutive newlines (a run of blank lines) → exactly two.
    static BLANKS: OnceLock<Regex> = OnceLock::new();

    let img_query =
        IMG_QUERY.get_or_init(|| Regex::new(r"(!\[[^\]]*\]\([^)\s?]+)\?[^)]*\)").unwrap());
    let span_open = SPAN_OPEN.get_or_init(|| Regex::new(r"(?s)<span\b[^>]*>").unwrap());
    let span_close = SPAN_CLOSE.get_or_init(|| Regex::new(r"</span>").unwrap());
    let mention = MENTION.get_or_init(|| {
        Regex::new(r"(?s)<mention-user\b[^>]*?/>|<mention-user\b[^>]*?>.*?</mention-user>").unwrap()
    });
    let br = BR.get_or_init(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
    let blanks = BLANKS.get_or_init(|| Regex::new(r"\n{3,}").unwrap());

    let s = img_query.replace_all(md, "$1)");
    let s = mention.replace_all(&s, "");
    let s = span_open.replace_all(&s, "");
    let s = span_close.replace_all(&s, "");
    let s = br.replace_all(&s, "\n");
    let s = blanks.replace_all(&s, "\n\n");
    s.trim().to_string()
}

/// Pretty-printed JSON body for one Notion page. We persist the *full*
/// Composio response payload (not just the title) so the chunked content
/// retains enough context for retrieval — Notion pages don't have a
/// natural single-string canonical body the way Slack messages do.
/// Render the canonical document body for a Notion page.
///
/// When `markdown` is `Some` (the page's rendered body, fetched via
/// `NOTION_GET_PAGE_MARKDOWN`), it is the primary content — the actual
/// paragraphs, headings, lists, and *body tables* a free-form page contains.
/// The `FETCH_DATA` metadata/properties JSON is appended after it so the
/// page's structured fields (status, assignee, due date — the real data for
/// database pages, which the markdown body does NOT include) stay searchable.
///
/// When `markdown` is `None` (no body fetched, or the page had none), we fall
/// back to the metadata-only body — the prior behaviour.
fn render_page_body(title: &str, page: &Value, markdown: Option<&str>) -> String {
    let pretty = serde_json::to_string_pretty(page).unwrap_or_else(|_| "{}".to_string());
    match markdown {
        Some(md) if !md.trim().is_empty() => {
            format!("# {title}\n\n{md}\n\n---\n\n```json\n{pretty}\n```\n")
        }
        _ => format!("# {title}\n\n```json\n{pretty}\n```\n"),
    }
}

/// Parse a Notion `last_edited_time` (ISO 8601 / RFC 3339) into a
/// `DateTime<Utc>`, falling back to `Utc::now()` on failure so the
/// pipeline still gets a valid timestamp.
fn parse_edited_time(raw: Option<&str>) -> DateTime<Utc> {
    raw.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

/// Ingest one Notion page revision into the memory tree (non-destructive,
/// versioned).
///
/// Caller (the provider's `sync` loop) owns the edit-detection / dedup
/// state-machine (`SyncState::synced_ids` keyed by `{page_id}@{edited_time}`)
/// — this function trusts it is only called for a revision the caller wants
/// to admit.
///
/// Unlike the pre-versioning behaviour (which deleted the prior chunks
/// before re-ingesting), an edit is now **non-destructive**: the page's
/// `last_edited_time` is used as the document version, the source-ingest gate
/// is keyed by `{source_id}@{version}` (so a new revision is admitted
/// alongside the old one), and a [`JobKind::SealDocument`] job is enqueued to
/// build this revision's per-document subtree and merge its doc-root into the
/// connection tree. Older revisions stay in place; the retrieval layer
/// surfaces only the latest version per document.
///
/// Returns the number of chunks the pipeline wrote for this revision.
pub async fn ingest_page_into_memory_tree(
    config: &Config,
    connection_id: &str,
    page_id: &str,
    title: &str,
    edited_time: Option<&str>,
    page: &Value,
    markdown_body: Option<&str>,
) -> Result<usize> {
    let source_id = notion_source_id(connection_id, page_id);
    let modified_at = parse_edited_time(edited_time);
    // Document version = Notion `last_edited_time` in epoch-ms. Drives both
    // the versioned ingest gate and the per-doc subtree's `version_ms` tag
    // used for read-time latest-wins.
    let version_ms = modified_at.timestamp_millis();
    // Prefer the rendered page body (NOTION_GET_PAGE_MARKDOWN) when present;
    // fall back to metadata-only when the caller didn't fetch it. Strip the
    // noise (signed image URLs, `<span>`/`<mention-user>` tags, `<br>`) from the
    // body markdown before rendering — the metadata JSON path is untouched.
    let cleaned = markdown_body.map(clean_notion_markdown);
    let body = render_page_body(title, page, cleaned.as_deref());
    let source_ref = Some(format!("notion://page/{page_id}"));

    let doc = DocumentInput {
        provider: NOTION_PLATFORM.to_string(),
        title: title.to_string(),
        body,
        modified_at,
        source_ref,
    };
    let tags: Vec<String> = DEFAULT_TAGS.iter().map(|s| s.to_string()).collect();
    let owner = notion_source_scope(connection_id);
    let path_scope = Some(owner.clone());

    let result = ingest_pipeline::ingest_document_versioned(
        config,
        &source_id,
        &owner,
        tags,
        doc,
        path_scope,
        Some(version_ms),
    )
    .await
    .map_err(|err| {
        // `{err:#}` keeps the anyhow context chain so provider.rs's
        // `tracing::warn!(error = %e)` shows the underlying cause.
        anyhow::anyhow!("ingest_document failed for {source_id}: {err:#}")
    })?;

    if result.already_ingested {
        // This exact revision was already ingested (provider dedup normally
        // prevents reaching here). Nothing new to seal.
        tracing::debug!(
            connection_id = %connection_id,
            page_id = %page_id,
            version_ms,
            "[composio:notion] ingest: revision already ingested — skipping seal"
        );
        return Ok(0);
    }

    // Enqueue the per-document seal: it rolls this revision's chunks up to a
    // single doc-root (tagged with `version_ms`) and merges it into the
    // connection tree. Forward-only — the prior revision's doc-root is left
    // untouched; retrieval filters to the latest version per document.
    if !result.chunk_ids.is_empty() {
        let payload = SealDocumentPayload {
            tree_scope: owner.clone(),
            doc_id: source_id.clone(),
            version_ms: Some(version_ms),
            chunk_ids: result.chunk_ids.clone(),
        };
        match NewJob::seal_document(&payload).and_then(|job| enqueue(config, &job)) {
            Ok(_) => wake_workers(),
            Err(e) => tracing::warn!(
                connection_id = %connection_id,
                page_id = %page_id,
                "[composio:notion] ingest: failed to enqueue seal_document: {e:#}"
            ),
        }
    }

    tracing::debug!(
        connection_id = %connection_id,
        page_id = %page_id,
        chunks_written = result.chunks_written,
        version_ms,
        "[composio:notion] ingest: page persisted (versioned)"
    );
    Ok(result.chunks_written)
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

    #[test]
    fn notion_source_scope_is_connection_level() {
        assert_eq!(notion_source_scope("conn-1"), "notion:conn-1");
        assert_eq!(notion_source_scope("conn-2"), "notion:conn-2");
        assert_eq!(
            notion_source_scope("conn-1"),
            notion_source_scope("conn-1"),
            "scope must stay stable across pages in one connection"
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
        // Metadata-only (no markdown body): title header + pretty JSON.
        let body = render_page_body("Phoenix plan", &page, None);
        assert!(body.starts_with("# Phoenix plan\n"));
        assert!(body.contains("```json\n"));
        assert!(body.contains("\"id\": \"p-1\""));
        assert!(body.contains("\"url\": \"https://notion.so/p1\""));
    }

    /// With a markdown body present, render the body as the primary content
    /// AND keep the metadata JSON appended after a `---` separator, so
    /// free-form pages get real content while structured `properties` stay
    /// searchable.
    #[test]
    fn render_page_body_merges_markdown_then_metadata() {
        let page = json!({ "id": "p-1", "properties": { "Status": "Done" } });
        let md = "## Heading\n\nReal body text with a list:\n- one\n- two";
        let body = render_page_body("Phoenix plan", &page, Some(md));
        assert!(body.starts_with("# Phoenix plan\n"));
        assert!(body.contains("Real body text with a list"));
        assert!(body.contains("\n---\n")); // separator before metadata
        assert!(body.contains("```json")); // metadata still present
        assert!(body.contains("\"Status\": \"Done\""));
        // Body comes BEFORE the metadata block.
        let md_pos = body.find("Real body text").unwrap();
        let json_pos = body.find("```json").unwrap();
        assert!(
            md_pos < json_pos,
            "markdown body must precede metadata json"
        );
    }

    /// Empty/whitespace markdown falls back to metadata-only (a database row
    /// with no body blocks returns empty markdown).
    #[test]
    fn render_page_body_empty_markdown_falls_back_to_metadata_only() {
        let page = json!({ "id": "p-1" });
        let body = render_page_body("Phoenix plan", &page, Some("   \n  "));
        assert!(
            !body.contains("\n---\n"),
            "empty markdown → no body section"
        );
        assert!(body.contains("```json"));
    }

    /// Signed-image-URL query strings are stripped to bare URLs (keeping the
    /// `.png`), so the hundreds of rotating `X-Amz-*` tokens don't pollute the
    /// chunk body or defeat content-hash idempotency.
    #[test]
    fn clean_notion_markdown_strips_image_query() {
        let md = "![diagram](https://prod-files.s3.amazonaws.com/a/b/file.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIA%2F20260604&X-Amz-Signature=deadbeef)";
        let out = clean_notion_markdown(md);
        assert_eq!(
            out,
            "![diagram](https://prod-files.s3.amazonaws.com/a/b/file.png)"
        );
        assert!(out.contains(".png"), "the .png extension must be kept");
        assert!(!out.contains("X-Amz-"), "no signed-query noise: {out}");
    }

    /// `<span ...>text</span>` collapses to its inner text, dropping the tag and
    /// any `color=` / `discussion-urls=` attributes.
    #[test]
    fn clean_notion_markdown_collapses_spans() {
        let out = clean_notion_markdown(r#"<span color="gray">text</span>"#);
        assert_eq!(out, "text");

        // Attributes like discussion-urls are dropped along with the tags.
        let out2 = clean_notion_markdown(
            r#"before <span discussion-urls="x" color="blue">middle</span> after"#,
        );
        assert_eq!(out2, "before middle after");
    }

    /// `<mention-user .../>` tags are removed entirely (both self-closing and
    /// paired forms).
    #[test]
    fn clean_notion_markdown_removes_mentions() {
        let out = clean_notion_markdown(r#"hi <mention-user url="user://x"/> there"#);
        assert_eq!(out, "hi  there");
        assert!(!out.contains("mention-user"));

        let paired =
            clean_notion_markdown(r#"<mention-user id="u1">@Alice</mention-user> owns it"#);
        assert_eq!(paired, "owns it");
    }

    /// `<br>` / `<br/>` / `<br />` become newlines.
    #[test]
    fn clean_notion_markdown_converts_br_to_newline() {
        assert_eq!(
            clean_notion_markdown("line one<br>line two"),
            "line one\nline two"
        );
        assert_eq!(clean_notion_markdown("a<br/>b"), "a\nb");
        assert_eq!(clean_notion_markdown("a<br />b"), "a\nb");
    }

    /// Plain markdown (no noise patterns) passes through unchanged apart from
    /// the trailing-whitespace trim.
    #[test]
    fn clean_notion_markdown_leaves_plain_markdown_unchanged() {
        let md = "# Heading\n\nReal body text with a list:\n- one\n- two\n\n[link](https://example.com/page)";
        assert_eq!(clean_notion_markdown(md), md);
    }

    /// 3+ consecutive blank lines collapse to a single blank line (two `\n`).
    #[test]
    fn clean_notion_markdown_collapses_blank_runs() {
        assert_eq!(clean_notion_markdown("a\n\n\n\n\nb"), "a\n\nb");
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
        use crate::openhuman::memory_store::chunks::store::{
            count_chunks, get_chunk_content_path, is_source_ingested, list_chunks, ListChunksQuery,
        };
        use crate::openhuman::memory_store::chunks::types::SourceKind;

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-test";
        let page_id = "page-phoenix";
        let expected = notion_source_id(connection_id, page_id);
        let expected_scope = notion_source_scope(connection_id);
        let page = sample_page(page_id, "2026-05-28T10:00:00.000Z");

        let chunks_before = count_chunks(&cfg).expect("count_chunks before");

        let written = ingest_page_into_memory_tree(
            &cfg,
            connection_id,
            page_id,
            "Phoenix migration plan",
            Some("2026-05-28T10:00:00.000Z"),
            &page,
            None,
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

        let rows = list_chunks(
            &cfg,
            &ListChunksQuery {
                source_kind: Some(SourceKind::Document),
                source_id: Some(expected.clone()),
                limit: Some(1),
                ..Default::default()
            },
        )
        .expect("list chunks for notion page");
        let chunk = rows.first().expect("notion chunk should be listed");
        assert_eq!(chunk.metadata.source_id, expected.as_str());
        assert_eq!(
            chunk.metadata.path_scope.as_deref(),
            Some(expected_scope.as_str())
        );
        let content_path = get_chunk_content_path(&cfg, &chunk.id)
            .expect("get chunk content path")
            .expect("document chunk should have content path");
        assert!(
            content_path.starts_with("document/notion-conn-test/"),
            "content path should use connection-level scope, got {content_path}"
        );
        let body = std::fs::read_to_string(cfg.memory_tree_content_root().join(&content_path))
            .expect("read chunk file");
        assert!(body.contains("source/notion-conn-test"), "{body}");
        assert!(
            !body.contains("source/notion-conn-test-page-phoenix"),
            "source tag must not include page id: {body}"
        );

        // Source registration. Versioning keys the ingest gate by
        // `{source_id}@{version_ms}` (version_ms = last_edited_time epoch-ms)
        // so a later revision is admitted non-destructively — assert the
        // versioned key is claimed, not the bare source_id.
        let version_ms = parse_edited_time(Some("2026-05-28T10:00:00.000Z")).timestamp_millis();
        let gate_key = format!("{expected}@{version_ms}");
        let cfg_for_blocking = cfg.clone();
        let gate_for_task = gate_key.clone();
        let registered = tokio::task::spawn_blocking(move || {
            is_source_ingested(&cfg_for_blocking, SourceKind::Document, &gate_for_task)
                .unwrap_or(false)
        })
        .await
        .expect("source-check task join");
        assert!(
            registered,
            "versioned gate key {gate_key} must be registered in mem_tree_ingested_sources"
        );
    }

    /// Re-ingesting an edited page (same `(connection_id, page_id)`,
    /// different content + newer `last_edited_time`) is now
    /// **non-destructive + versioned**: the new revision is admitted via the
    /// `{source_id}@{version_ms}` gate key (not short-circuited), and the
    /// prior revision's chunks are LEFT IN PLACE. Read-time latest-wins
    /// surfaces only the newer version; the write path never deletes the old
    /// one. (Replaces the pre-versioning destructive `delete_chunks_by_source`
    /// behaviour.)
    #[tokio::test]
    async fn re_ingesting_edited_page_keeps_both_versions() {
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
            None,
        )
        .await
        .expect("first ingest");
        assert!(first > 0);
        let after_first = count_chunks(&cfg).expect("count after first");

        // Re-ingest with different body + newer edit time — must NOT
        // short-circuit (different version gate key) and must NOT delete the
        // prior revision's chunks.
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
            None,
        )
        .await
        .expect("second ingest");
        assert!(
            second > 0,
            "edited page must actually re-ingest, not silently no-op"
        );
        let after_second = count_chunks(&cfg).expect("count after second");

        // Non-destructive: the second revision's chunks are ADDED on top of
        // the first revision's, so the total grows rather than staying flat.
        assert!(
            after_second > after_first,
            "edited page must keep both versions (non-destructive): \
             after_first={after_first} after_second={after_second}"
        );
    }
}
