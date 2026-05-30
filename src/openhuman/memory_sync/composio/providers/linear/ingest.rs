//! Linear -> memory tree ingest plumbing.
//!
//! Converts one Linear issue payload into a memory_tree [`DocumentInput`]
//! and calls `ingest_document` so retrieval surfaces read the content from
//! `mem_tree_chunks` instead of the legacy `memory_docs` path.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::{self, IngestResult};
use crate::openhuman::memory_store::chunks::store::{delete_chunks_by_source, is_source_ingested};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;

/// Platform identifier embedded in Linear document metadata.
pub const LINEAR_PLATFORM: &str = "linear";

/// Stable tags attached to every Linear-ingested issue chunk.
pub const DEFAULT_TAGS: &[&str] = &["linear", "ingested"];

/// Build the memory-tree source id for one Linear issue in one connection.
pub(crate) fn linear_source_id(connection_id: &str, issue_id: &str) -> String {
    format!("linear:{connection_id}:{issue_id}")
}

/// Render the raw Linear issue payload as a markdown document body.
fn render_issue_body(title: &str, issue: &Value) -> String {
    let pretty = serde_json::to_string_pretty(issue).unwrap_or_else(|_| "{}".to_string());
    format!("# {title}\n\n```json\n{pretty}\n```\n")
}

/// Parse Linear's `updatedAt` timestamp, falling back to now on malformed input.
fn parse_updated_time(raw: Option<&str>) -> DateTime<Utc> {
    raw.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

/// Ingest one Linear issue into memory_tree and return the written chunk count.
///
/// Edited issues reuse the same `source_id`, so prior chunks are deleted before
/// re-ingest to avoid the document pipeline's duplicate-source short-circuit.
pub async fn ingest_issue_into_memory_tree(
    config: &Config,
    connection_id: &str,
    issue_id: &str,
    title: &str,
    updated_time: Option<&str>,
    issue: &Value,
) -> Result<usize> {
    let source_id = linear_source_id(connection_id, issue_id);

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
            "[composio:linear] ingest: re-ingest cleanup"
        );
    }

    let modified_at = parse_updated_time(updated_time);
    let body = render_issue_body(title, issue);
    let source_ref = Some(format!("linear://issue/{issue_id}"));
    let doc = DocumentInput {
        provider: LINEAR_PLATFORM.to_string(),
        title: title.to_string(),
        body,
        modified_at,
        source_ref,
    };
    let tags: Vec<String> = DEFAULT_TAGS.iter().map(|s| s.to_string()).collect();
    let owner = format!("linear:{connection_id}");

    match ingest_pipeline::ingest_document(config, &source_id, &owner, tags, doc).await {
        Ok(IngestResult {
            chunks_written,
            already_ingested,
            ..
        }) => {
            tracing::debug!(
                connection_id = %connection_id,
                issue_id = %issue_id,
                chunks_written,
                already_ingested,
                "[composio:linear] ingest: issue persisted"
            );
            Ok(chunks_written)
        }
        Err(err) => Err(anyhow::anyhow!(
            "ingest_document failed for {source_id}: {err:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::memory_store::chunks::types::SourceKind;
    use chrono::Utc;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().expect("tempdir");
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn sample_issue(issue_id: &str, updated_at: &str) -> Value {
        json!({
            "id": issue_id,
            "identifier": "ENG-42",
            "title": "Fix external LLM routing",
            "updatedAt": updated_at,
            "url": "https://linear.app/openhuman/issue/ENG-42/fix-external-llm-routing",
            "description": "Connected app tools vanish under external routing.",
            "state": { "name": "In Progress" },
            "team": { "key": "ENG", "name": "Engineering" },
            "assignee": { "name": "Alice" }
        })
    }

    #[test]
    fn linear_source_id_is_stable_and_namespaced() {
        let a = linear_source_id("conn-1", "issue-abc");
        let b = linear_source_id("conn-1", "issue-abc");
        assert_eq!(a, b);
        assert_eq!(a, "linear:conn-1:issue-abc");
        assert_ne!(a, linear_source_id("conn-2", "issue-abc"));
        assert_ne!(a, linear_source_id("conn-1", "issue-xyz"));
    }

    #[test]
    fn parse_updated_time_handles_valid_and_invalid_inputs() {
        let good = parse_updated_time(Some("2026-05-28T12:34:56.000Z"));
        assert_eq!(good.format("%Y-%m-%d").to_string(), "2026-05-28");

        let bad = parse_updated_time(Some("not-a-timestamp"));
        assert!((Utc::now() - bad).num_seconds().abs() < 5);

        let missing = parse_updated_time(None);
        assert!((Utc::now() - missing).num_seconds().abs() < 5);
    }

    #[test]
    fn render_issue_body_includes_title_header_and_pretty_json() {
        let issue = json!({
            "id": "issue-1",
            "identifier": "ENG-42",
            "title": "Fix external LLM routing"
        });
        let body = render_issue_body("Linear: Fix external LLM routing", &issue);
        assert!(body.starts_with("# Linear: Fix external LLM routing\n"));
        assert!(body.contains("```json\n"));
        assert!(body.contains("\"identifier\": \"ENG-42\""));
        assert!(body.contains("\"title\": \"Fix external LLM routing\""));
    }

    #[tokio::test]
    async fn ingest_issue_writes_to_memory_tree() {
        use crate::openhuman::memory_store::chunks::store::{count_chunks, is_source_ingested};

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-linear";
        let issue_id = "issue-routing";
        let issue = sample_issue(issue_id, "2026-05-28T10:00:00.000Z");
        let chunks_before = count_chunks(&cfg).expect("count_chunks before");

        let written = ingest_issue_into_memory_tree(
            &cfg,
            connection_id,
            issue_id,
            "Linear: Fix external LLM routing",
            Some("2026-05-28T10:00:00.000Z"),
            &issue,
        )
        .await
        .expect("ingest_issue_into_memory_tree");

        assert!(written > 0, "Linear ingest must write chunks");
        let chunks_after = count_chunks(&cfg).expect("count_chunks after");
        assert!(
            chunks_after > chunks_before,
            "ingest must populate mem_tree_chunks (#2885)"
        );

        let cfg_for_blocking = cfg.clone();
        let expected = linear_source_id(connection_id, issue_id);
        let registered = tokio::task::spawn_blocking(move || {
            is_source_ingested(&cfg_for_blocking, SourceKind::Document, &expected).unwrap_or(false)
        })
        .await
        .expect("source-check task join");
        assert!(registered, "source_id must be registered");
    }

    #[tokio::test]
    async fn re_ingesting_edited_issue_replaces_prior_chunks() {
        use crate::openhuman::memory_store::chunks::store::count_chunks;

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-edit";
        let issue_id = "issue-edit";

        let v1 = sample_issue(issue_id, "2026-05-28T10:00:00.000Z");
        let first = ingest_issue_into_memory_tree(
            &cfg,
            connection_id,
            issue_id,
            "Linear: Fix external LLM routing",
            Some("2026-05-28T10:00:00.000Z"),
            &v1,
        )
        .await
        .expect("first ingest");
        assert!(first > 0);
        let after_first = count_chunks(&cfg).expect("count after first");

        let v2 = json!({
            "id": issue_id,
            "identifier": "ENG-42",
            "title": "Fix external LLM routing",
            "updatedAt": "2026-05-29T10:00:00.000Z",
            "description": "Updated: external LLM routing now keeps connected app tools visible.",
            "state": { "name": "Done" }
        });
        let second = ingest_issue_into_memory_tree(
            &cfg,
            connection_id,
            issue_id,
            "Linear: Fix external LLM routing",
            Some("2026-05-29T10:00:00.000Z"),
            &v2,
        )
        .await
        .expect("second ingest");
        assert!(second > 0);
        let after_second = count_chunks(&cfg).expect("count after second");

        assert!(
            after_second.abs_diff(after_first) <= 1,
            "edited issue must replace prior chunks, not append duplicates"
        );
    }
}
