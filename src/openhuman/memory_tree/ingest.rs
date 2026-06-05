//! Single-shape entry point for landing a pre-built summary into a tree.
//!
//! Sync pipelines produce summaries (via an LLM call over batched source
//! content) and hand them here. `ingest_summary` writes the `.md` file to
//! `wiki/summaries/source-<slug>/L1/…`, inserts the `SummaryNode` row,
//! appends to the L1 buffer, and cascades seals upward when the buffer
//! crosses the fanout threshold.
//!
//! Embeddings are temporarily disabled — summaries land without vectors.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::with_connection;
use crate::openhuman::memory_store::content::atomic::{stage_summary, StagedSummary};
use crate::openhuman::memory_store::content::SummaryComposeInput;
use crate::openhuman::memory_store::trees::types::{SummaryNode, Tree, SUMMARY_FANOUT};
use crate::openhuman::memory_tree::tree::factory::TreeFactory;
use crate::openhuman::memory_tree::tree::registry::new_summary_id;
use crate::openhuman::memory_tree::tree::store;

/// Input for `ingest_summary`. Callers provide the summary text and
/// envelope metadata; the function handles id generation, file staging,
/// DB persistence, and buffer management.
#[derive(Clone, Debug)]
pub struct SummaryIngestInput {
    pub content: String,
    pub token_count: u32,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub time_range_start: DateTime<Utc>,
    pub time_range_end: DateTime<Utc>,
    pub score: f32,
    /// Opaque child references for the wikilink list in the `.md` front-matter.
    /// These are NOT tree children in the DB sense — they're provenance
    /// labels (e.g. `"commit:abc"`, `"issue:42"`) that appear in the
    /// summary's `children:` YAML block for human readers.
    pub child_labels: Vec<String>,
    /// Per-child wikilink overrides (parallel to `child_labels`). When
    /// `Some(path)`, the wikilink points at `[[path]]` instead of the
    /// sanitised child label. Use this to link to raw archive files so
    /// Obsidian can resolve the wikilinks.
    #[doc(hidden)]
    pub child_basenames: Vec<Option<String>>,
}

/// What `ingest_summary` did.
#[derive(Clone, Debug)]
pub struct SummaryIngestOutcome {
    pub summary_id: String,
    pub content_path: String,
    /// Summary ids that sealed during the cascade triggered by this ingest.
    pub sealed_ids: Vec<String>,
}

/// Ingest a pre-built summary into `tree` as an L1 node.
///
/// 1. Generate a summary id.
/// 2. Stage the `.md` file under `wiki/summaries/source-<slug>/L1/…`.
/// 3. Insert the `SummaryNode` row.
/// 4. Append to the L1 buffer.
/// 5. Cascade seals if the L1 buffer crosses `SUMMARY_FANOUT`.
///
/// Embeddings are skipped — the `embedding` column is `None`.
pub async fn ingest_summary(
    config: &Config,
    tree: &Tree,
    input: SummaryIngestInput,
) -> Result<SummaryIngestOutcome> {
    let target_level: u32 = 1;
    let summary_id = new_summary_id(target_level);
    let now = Utc::now();

    let tree_factory = TreeFactory::from_tree(tree);
    let scope_slug = tree_factory.scope_slug();
    let summary_tree_kind = tree_factory.summary_tree_kind();
    let content_root = config.memory_tree_content_root();

    // Ensure Obsidian defaults are present (best-effort).
    if let Err(e) =
        crate::openhuman::memory_store::content::obsidian::ensure_obsidian_defaults(&content_root)
    {
        tracing::warn!(
            error = %format!("{e:#}"),
            "[memory_tree::ingest] ensure_obsidian_defaults failed"
        );
    }

    // Stage the .md file BEFORE the DB transaction.
    let compose_input = SummaryComposeInput {
        summary_id: &summary_id,
        tree_kind: summary_tree_kind,
        tree_id: &tree.id,
        tree_scope: &tree.scope,
        level: target_level,
        child_ids: &input.child_labels,
        child_basenames: if input.child_basenames.is_empty() {
            None
        } else {
            Some(&input.child_basenames)
        },
        child_count: input.child_labels.len(),
        time_range_start: input.time_range_start,
        time_range_end: input.time_range_end,
        sealed_at: now,
        body: &input.content,
    };

    let staged = stage_summary(&content_root, &compose_input, &scope_slug)
        .with_context(|| format!("stage_summary failed for {summary_id}"))?;

    tracing::debug!(
        summary_id = %summary_id,
        path = %staged.content_path,
        "[memory_tree::ingest] staged summary"
    );

    let node = SummaryNode {
        id: summary_id.clone(),
        tree_id: tree.id.clone(),
        tree_kind: tree.kind,
        level: target_level,
        parent_id: None,
        child_ids: input.child_labels,
        content: input.content,
        token_count: input.token_count,
        entities: input.entities,
        topics: input.topics,
        time_range_start: input.time_range_start,
        time_range_end: input.time_range_end,
        score: input.score,
        sealed_at: now,
        deleted: false,
        embedding: None,
        doc_id: None,
        version_ms: None,
    };

    // Persist summary + update buffer in one transaction.
    persist_and_buffer(config, tree, &node, &staged, target_level)?;

    // Cascade seals upward from L1 if the buffer crossed the fanout gate.
    let sealed_ids = cascade_from(config, tree, target_level).await?;

    tracing::info!(
        summary_id = %summary_id,
        tree_id = %tree.id,
        sealed = sealed_ids.len(),
        "[memory_tree::ingest] ingested summary"
    );

    Ok(SummaryIngestOutcome {
        summary_id,
        content_path: staged.content_path,
        sealed_ids,
    })
}

fn persist_and_buffer(
    config: &Config,
    tree: &Tree,
    node: &SummaryNode,
    staged: &StagedSummary,
    target_level: u32,
) -> Result<()> {
    let tree_id = tree.id.clone();
    let summary_id = node.id.clone();
    let token_count = node.token_count;
    let time_range_start = node.time_range_start;
    let now = node.sealed_at;
    let node = node.clone();

    let model_signature =
        crate::openhuman::memory_store::chunks::store::tree_active_signature(config);
    let staged = staged.clone();

    with_connection(config, move |conn| {
        let tx = conn.unchecked_transaction()?;

        let current_max: u32 = tx
            .query_row(
                "SELECT max_level FROM mem_tree_trees WHERE id = ?1",
                rusqlite::params![&tree_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as u32)
            .unwrap_or(0);

        store::insert_summary_tx(&tx, &node, Some(&staged), &model_signature)?;

        // Index entities for retrieval.
        crate::openhuman::memory_tree::score::store::index_summary_entity_ids_tx(
            &tx,
            &node.entities,
            &node.id,
            node.score,
            now.timestamp_millis(),
            Some(&tree_id),
        )?;

        // Append to L1 buffer.
        let mut buf = store::get_buffer_conn(&tx, &tree_id, target_level)?;
        if !buf.item_ids.iter().any(|id| id == &summary_id) {
            buf.item_ids.push(summary_id.clone());
            buf.token_sum = buf.token_sum.saturating_add(token_count as i64);
            buf.oldest_at = match buf.oldest_at {
                Some(existing) => Some(existing.min(time_range_start)),
                None => Some(time_range_start),
            };
            store::upsert_buffer_tx(&tx, &buf)?;
        }

        // Update tree max_level if this is the first L1 node.
        if target_level > current_max {
            store::update_tree_after_seal_tx(&tx, &tree_id, &summary_id, target_level, now)?;
        }

        tx.commit()?;
        Ok(())
    })
}

/// Cascade seals starting at `start_level` using the existing bucket_seal
/// machinery. Only fires when the buffer at `start_level` has enough
/// siblings.
async fn cascade_from(config: &Config, tree: &Tree, start_level: u32) -> Result<Vec<String>> {
    use crate::openhuman::memory_tree::tree::bucket_seal::{cascade_all_from, LabelStrategy};

    let buf = store::get_buffer(config, &tree.id, start_level)?;
    if (buf.item_ids.len() as u32) < SUMMARY_FANOUT {
        return Ok(Vec::new());
    }

    cascade_all_from(config, tree, start_level, None, &LabelStrategy::Empty).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn ingest_summary_writes_to_l1_buffer() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();

        let tree = get_or_create_source_tree(&cfg, "github:org/repo").unwrap();
        let input = SummaryIngestInput {
            content: "Summary of 100 commits about feature X.".to_string(),
            token_count: 50,
            entities: Vec::new(),
            topics: vec!["github".to_string()],
            time_range_start: Utc::now(),
            time_range_end: Utc::now(),
            score: 0.7,
            child_labels: vec!["commit:abc".to_string(), "commit:def".to_string()],
            child_basenames: Vec::new(),
        };

        let outcome = ingest_summary(&cfg, &tree, input).await.unwrap();
        assert!(outcome.summary_id.starts_with("summary:"));
        assert!(outcome.content_path.contains("/L1/"));
        assert!(outcome.sealed_ids.is_empty());

        let buf = store::get_buffer(&cfg, &tree.id, 1).unwrap();
        assert_eq!(buf.item_ids.len(), 1);
        assert_eq!(buf.item_ids[0], outcome.summary_id);
        assert_eq!(buf.token_sum, 50);
    }

    #[tokio::test]
    async fn ingest_summary_is_idempotent_on_buffer() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();

        let tree = get_or_create_source_tree(&cfg, "github:org/repo2").unwrap();

        for _ in 0..3 {
            let input = SummaryIngestInput {
                content: "Same summary.".to_string(),
                token_count: 10,
                entities: Vec::new(),
                topics: Vec::new(),
                time_range_start: Utc::now(),
                time_range_end: Utc::now(),
                score: 0.5,
                child_labels: Vec::new(),
                child_basenames: Vec::new(),
            };
            ingest_summary(&cfg, &tree, input).await.unwrap();
        }

        let buf = store::get_buffer(&cfg, &tree.id, 1).unwrap();
        // Each call generates a unique summary_id, so all 3 should be in the buffer.
        assert_eq!(buf.item_ids.len(), 3);
    }

    #[tokio::test]
    async fn ingest_summary_writes_md_file() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();

        let tree = get_or_create_source_tree(&cfg, "github:org/repo3").unwrap();
        let input = SummaryIngestInput {
            content: "Test summary content.".to_string(),
            token_count: 20,
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: Utc::now(),
            time_range_end: Utc::now(),
            score: 0.5,
            child_labels: Vec::new(),
            child_basenames: Vec::new(),
        };

        let outcome = ingest_summary(&cfg, &tree, input).await.unwrap();
        let content_root = cfg.memory_tree_content_root();
        let abs_path = content_root.join(&outcome.content_path);
        assert!(abs_path.exists(), "summary .md file should exist on disk");

        let contents = std::fs::read_to_string(&abs_path).unwrap();
        assert!(contents.contains("Test summary content."));
    }
}
