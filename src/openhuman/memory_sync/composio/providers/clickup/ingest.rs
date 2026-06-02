//! ClickUp -> memory tree ingest plumbing.
//!
//! Converts one ClickUp task payload into a memory_tree [`DocumentInput`]
//! and calls `ingest_document` so retrieval surfaces read the content from
//! `mem_tree_chunks` instead of the legacy `memory_docs` path (issue #2885).
//!
//! ClickUp differs from the Linear/Notion providers in one way: its
//! `date_updated` field is a Unix epoch **milliseconds** value rendered as a
//! string (e.g. `"1779962400000"`), not an RFC3339 timestamp — so
//! [`parse_updated_time`] parses milliseconds rather than calling
//! `DateTime::parse_from_rfc3339`.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::{self, IngestResult};
use crate::openhuman::memory_store::chunks::store::{delete_chunks_by_source, is_source_ingested};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;

/// Platform identifier embedded in ClickUp document metadata.
pub const CLICKUP_PLATFORM: &str = "clickup";

/// Stable tags attached to every ClickUp-ingested task chunk.
pub const DEFAULT_TAGS: &[&str] = &["clickup", "ingested"];

/// Build the memory-tree source id for one ClickUp task in one connection.
pub(crate) fn clickup_source_id(connection_id: &str, task_id: &str) -> String {
    format!("clickup:{connection_id}:{task_id}")
}

/// Render the raw ClickUp task payload as a markdown document body.
fn render_task_body(title: &str, task: &Value) -> String {
    let pretty = serde_json::to_string_pretty(task).unwrap_or_else(|_| "{}".to_string());
    format!("# {title}\n\n```json\n{pretty}\n```\n")
}

/// Parse ClickUp's `date_updated` (Unix epoch **milliseconds** as a string),
/// falling back to now on missing/malformed input.
fn parse_updated_time(raw: Option<&str>) -> DateTime<Utc> {
    raw.and_then(|s| s.trim().parse::<i64>().ok())
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .unwrap_or_else(Utc::now)
}

/// Ingest one ClickUp task into memory_tree and return the written chunk count.
///
/// Edited tasks reuse the same `source_id`, so prior chunks are deleted before
/// re-ingest to avoid the document pipeline's duplicate-source short-circuit.
pub async fn ingest_task_into_memory_tree(
    config: &Config,
    connection_id: &str,
    task_id: &str,
    title: &str,
    updated_time: Option<&str>,
    task: &Value,
) -> Result<usize> {
    let source_id = clickup_source_id(connection_id, task_id);

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
            task_id = %task_id,
            removed_chunks = removed,
            "[composio:clickup] ingest: re-ingest cleanup"
        );
    }

    let modified_at = parse_updated_time(updated_time);
    let body = render_task_body(title, task);
    let source_ref = Some(format!("clickup://task/{task_id}"));
    let doc = DocumentInput {
        provider: CLICKUP_PLATFORM.to_string(),
        title: title.to_string(),
        body,
        modified_at,
        source_ref,
    };
    let tags: Vec<String> = DEFAULT_TAGS.iter().map(|s| s.to_string()).collect();
    let owner = format!("clickup:{connection_id}");

    match ingest_pipeline::ingest_document(config, &source_id, &owner, tags, doc).await {
        Ok(IngestResult {
            chunks_written,
            already_ingested,
            ..
        }) => {
            tracing::debug!(
                connection_id = %connection_id,
                task_id = %task_id,
                chunks_written,
                already_ingested,
                "[composio:clickup] ingest: task persisted"
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
    use chrono::{TimeZone, Utc};
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

    fn sample_task(task_id: &str, date_updated: &str) -> Value {
        json!({
            "id": task_id,
            "name": "Fix external LLM routing",
            "date_updated": date_updated,
            "url": "https://app.clickup.com/t/abc123",
            "status": { "status": "in progress" },
            "text_content": "Connected app tools vanish under external routing.",
            "assignees": [{ "username": "Alice" }]
        })
    }

    #[test]
    fn clickup_source_id_is_stable_and_namespaced() {
        let a = clickup_source_id("conn-1", "task-abc");
        let b = clickup_source_id("conn-1", "task-abc");
        assert_eq!(a, b);
        assert_eq!(a, "clickup:conn-1:task-abc");
        assert_ne!(a, clickup_source_id("conn-2", "task-abc"));
        assert_ne!(a, clickup_source_id("conn-1", "task-xyz"));
    }

    #[test]
    fn parse_updated_time_handles_epoch_millis_and_invalid_inputs() {
        // ClickUp sends epoch milliseconds as a string. Build the expected
        // instant and round-trip through its millisecond representation so we
        // never hand-compute the magic number.
        let expected = Utc.with_ymd_and_hms(2026, 5, 28, 10, 0, 0).unwrap();
        let ms = expected.timestamp_millis().to_string();
        let good = parse_updated_time(Some(&ms));
        assert_eq!(good, expected);

        // Leading/trailing whitespace must still parse.
        let padded = parse_updated_time(Some(&format!("  {ms}  ")));
        assert_eq!(padded, expected);

        let bad = parse_updated_time(Some("not-a-timestamp"));
        assert!((Utc::now() - bad).num_seconds().abs() < 5);

        // An RFC3339 string is *not* valid ClickUp input — it must fall back.
        let rfc = parse_updated_time(Some("2026-05-28T10:00:00.000Z"));
        assert!((Utc::now() - rfc).num_seconds().abs() < 5);

        let missing = parse_updated_time(None);
        assert!((Utc::now() - missing).num_seconds().abs() < 5);
    }

    #[test]
    fn render_task_body_includes_title_header_and_pretty_json() {
        let task = json!({
            "id": "task-1",
            "name": "Fix external LLM routing",
            "date_updated": "1779962400000"
        });
        let body = render_task_body("ClickUp: Fix external LLM routing", &task);
        assert!(body.starts_with("# ClickUp: Fix external LLM routing\n"));
        assert!(body.contains("```json\n"));
        assert!(body.contains("\"name\": \"Fix external LLM routing\""));
        assert!(body.contains("\"date_updated\": \"1779962400000\""));
    }

    #[tokio::test]
    async fn ingest_task_writes_to_memory_tree() {
        use crate::openhuman::memory_store::chunks::store::{count_chunks, is_source_ingested};

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-clickup";
        let task_id = "task-routing";
        let task = sample_task(task_id, "1779962400000");
        let chunks_before = count_chunks(&cfg).expect("count_chunks before");

        let written = ingest_task_into_memory_tree(
            &cfg,
            connection_id,
            task_id,
            "ClickUp: Fix external LLM routing",
            Some("1779962400000"),
            &task,
        )
        .await
        .expect("ingest_task_into_memory_tree");

        assert!(written > 0, "ClickUp ingest must write chunks");
        let chunks_after = count_chunks(&cfg).expect("count_chunks after");
        assert!(
            chunks_after > chunks_before,
            "ingest must populate mem_tree_chunks (#2885)"
        );

        let cfg_for_blocking = cfg.clone();
        let expected = clickup_source_id(connection_id, task_id);
        let registered = tokio::task::spawn_blocking(move || {
            is_source_ingested(&cfg_for_blocking, SourceKind::Document, &expected).unwrap_or(false)
        })
        .await
        .expect("source-check task join");
        assert!(registered, "source_id must be registered");
    }

    #[tokio::test]
    async fn re_ingesting_edited_task_replaces_prior_chunks() {
        use crate::openhuman::memory_store::chunks::store::count_chunks;

        let (_tmp, cfg) = test_config();
        let connection_id = "conn-edit";
        let task_id = "task-edit";

        let v1 = sample_task(task_id, "1779962400000");
        let first = ingest_task_into_memory_tree(
            &cfg,
            connection_id,
            task_id,
            "ClickUp: Fix external LLM routing",
            Some("1779962400000"),
            &v1,
        )
        .await
        .expect("first ingest");
        assert!(first > 0);
        let after_first = count_chunks(&cfg).expect("count after first");

        let v2 = json!({
            "id": task_id,
            "name": "Fix external LLM routing",
            "date_updated": "1780048800000",
            "text_content": "Updated: external LLM routing now keeps connected app tools visible.",
            "status": { "status": "complete" }
        });
        let second = ingest_task_into_memory_tree(
            &cfg,
            connection_id,
            task_id,
            "ClickUp: Fix external LLM routing",
            Some("1780048800000"),
            &v2,
        )
        .await
        .expect("second ingest");
        assert!(second > 0);
        let after_second = count_chunks(&cfg).expect("count after second");

        assert!(
            after_second.abs_diff(after_first) <= 1,
            "edited task must replace prior chunks, not append duplicates"
        );
    }
}
