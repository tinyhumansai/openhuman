//! GitHub repo sync pipeline.
//!
//! Git clone → read commits → fetch issues/PRs from API → batch items
//! into ~50k-token groups → summarise each batch → ingest into the
//! memory tree via `memory_tree::ingest::ingest_summary`.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sync::{emit_sync_stage, MemorySyncStage, MemorySyncTrigger};
use crate::openhuman::memory::tree_source::get_or_create_source_tree;
use crate::openhuman::memory_sources::readers;
use crate::openhuman::memory_sources::readers::github;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceContent, SourceKind};
use crate::openhuman::memory_store::content::raw::{self as raw_store, RawItem};
use crate::openhuman::memory_store::trees::types::TreeKind;
use crate::openhuman::memory_store::trees::types::INPUT_TOKEN_BUDGET;
use crate::openhuman::memory_sync::sources::audit::{
    append_audit_entry, RealCostAccumulator, SyncAuditEntry,
};
use crate::openhuman::memory_sync::traits::{SyncOutcome, SyncPipeline, SyncPipelineKind};
use crate::openhuman::memory_tree::ingest::{ingest_summary, SummaryIngestInput};
use crate::openhuman::memory_tree::summarise::{
    fallback_summary, summarise, SummaryContext, SummaryInput,
};

pub struct GithubSourcePipeline {
    source: MemorySourceEntry,
}

impl GithubSourcePipeline {
    pub fn new(source: MemorySourceEntry) -> Self {
        Self { source }
    }
}

#[async_trait]
impl SyncPipeline for GithubSourcePipeline {
    fn id(&self) -> &str {
        &self.source.id
    }

    fn kind(&self) -> SyncPipelineKind {
        SyncPipelineKind::Workspace
    }

    async fn init(&self, _config: &Config) -> anyhow::Result<()> {
        Ok(())
    }

    async fn tick(&self, config: &Config) -> anyhow::Result<SyncOutcome> {
        run_github_sync(&self.source, config).await
    }
}

/// Run a full GitHub source sync: clone/fetch → list items → read all →
/// batch into 50k-token groups → summarise each → ingest into tree.
pub async fn run_github_sync(
    source: &MemorySourceEntry,
    config: &Config,
) -> anyhow::Result<SyncOutcome> {
    let start = std::time::Instant::now();
    let source_id = &source.id;
    let kind_str = source.kind.as_str();

    let repo_scope = source
        .url
        .as_deref()
        .and_then(github::repo_chunk_scope)
        .ok_or_else(|| anyhow::anyhow!("github source missing url for repo scope"))?;

    let raw_source_id = source
        .url
        .as_deref()
        .and_then(github::repo_archive_source_id);

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some(kind_str),
        Some(source_id),
        Some("listing items".to_string()),
    );

    let reader = readers::reader_for(&SourceKind::GithubRepo);
    let items = reader
        .list_items(source, config)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let total = items.len();

    tracing::debug!(
        source_id = %source_id,
        total = total,
        "[memory_sync:github] listed items"
    );

    if total == 0 {
        return Ok(SyncOutcome {
            records_ingested: 0,
            more_pending: false,
            note: Some("no items found".to_string()),
        });
    }

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Ingesting,
        Some(kind_str),
        Some(source_id),
        Some(format!("reading {total} items")),
    );

    let content_root = config.memory_tree_content_root();
    let mut inputs: Vec<SummaryInput> = Vec::with_capacity(total);
    let mut child_labels: Vec<String> = Vec::with_capacity(total);
    let mut child_basenames: Vec<Option<String>> = Vec::with_capacity(total);

    for (idx, item) in items.iter().enumerate() {
        let content = match reader.read_item(source, &item.id, config).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    item_id = %item.id,
                    error = %e,
                    "[memory_sync:github] skipping item — read failed"
                );
                continue;
            }
        };

        if let Some(ref raw_sid) = raw_source_id {
            write_raw_archive(
                raw_sid,
                &item.id,
                item.updated_at_ms,
                &content,
                &content_root,
            );
        }

        // Compute the raw archive relative path for the wikilink.
        let raw_path = raw_source_id.as_deref().and_then(|raw_sid| {
            let (kind, uid) = github::raw_archive_coords(&item.id)?;
            let created_at_ms = item.updated_at_ms.unwrap_or(0);
            let rel = raw_store::raw_rel_path(raw_sid, kind, created_at_ms, &uid);
            Some(rel.strip_suffix(".md").unwrap_or(&rel).to_string())
        });

        let token_count = (content.body.len() / 4).max(1) as u32;
        let priority = item_is_high_priority(&item.id, &content);
        let ts = item
            .updated_at_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
            .unwrap_or_else(chrono::Utc::now);

        inputs.push(SummaryInput {
            id: item.id.clone(),
            content: content.body,
            token_count,
            entities: Vec::new(),
            topics: vec!["memory_sources".to_string(), kind_str.to_string()],
            time_range_start: ts,
            time_range_end: ts,
            score: if priority { 0.8 } else { 0.5 },
        });
        child_labels.push(item.id.clone());
        child_basenames.push(raw_path);

        if (idx + 1) % 100 == 0 || idx + 1 == total {
            emit_sync_stage(
                MemorySyncTrigger::Manual,
                MemorySyncStage::Ingesting,
                Some(kind_str),
                Some(source_id),
                Some(format!("{}/{total} read", idx + 1)),
            );
        }
    }

    if inputs.is_empty() {
        return Ok(SyncOutcome {
            records_ingested: 0,
            more_pending: false,
            note: Some("all items failed to read".to_string()),
        });
    }

    let tree = get_or_create_source_tree(config, &repo_scope)
        .map_err(|e| anyhow::anyhow!("get_or_create_source_tree: {e:#}"))?;

    let batches =
        batch_by_token_budget(&inputs, &child_labels, &child_basenames, INPUT_TOKEN_BUDGET);
    let batch_count = batches.len();
    let input_count = inputs.len();

    tracing::info!(
        source_id = %source_id,
        items = input_count,
        batches = batch_count,
        "[memory_sync:github] summarising in {} batch(es)",
        batch_count
    );

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Ingesting,
        Some(kind_str),
        Some(source_id),
        Some(format!(
            "summarising {input_count} items in {batch_count} batch(es)"
        )),
    );

    // Token/charge accounting across the run. The estimate (`body.len() / 4`
    // heuristic) is always summed; provider-reported figures only replace it
    // when every batch reported them (issue #3110). See `RealCostAccumulator`.
    let mut cost = RealCostAccumulator::new();

    for (batch_idx, (batch_inputs, batch_labels, batch_basenames)) in
        batches.into_iter().enumerate()
    {
        let batch_input_tokens: u64 = batch_inputs.iter().map(|i| i.token_count as u64).sum();

        let ctx = SummaryContext {
            tree_id: &tree.id,
            tree_kind: TreeKind::Source,
            target_level: 1,
            token_budget: 5_000,
        };

        let output = match summarise(config, &batch_inputs, &ctx).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    batch = batch_idx,
                    "[memory_sync:github] summarise failed, using fallback"
                );
                fallback_summary(&batch_inputs, ctx.token_budget)
            }
        };

        cost.add_batch(
            batch_input_tokens,
            output.token_count as u64,
            output.input_tokens,
            output.output_tokens,
            output.charged_amount_usd,
        );

        let time_start = batch_inputs
            .iter()
            .map(|i| i.time_range_start)
            .min()
            .unwrap_or_else(chrono::Utc::now);
        let time_end = batch_inputs
            .iter()
            .map(|i| i.time_range_end)
            .max()
            .unwrap_or_else(chrono::Utc::now);
        let max_score = batch_inputs.iter().map(|i| i.score).fold(0.0f32, f32::max);

        let ingest_input = SummaryIngestInput {
            content: output.content,
            token_count: output.token_count,
            entities: Vec::new(),
            topics: vec!["memory_sources".to_string(), kind_str.to_string()],
            time_range_start: time_start,
            time_range_end: time_end,
            score: max_score,
            child_labels: batch_labels,
            child_basenames: batch_basenames,
        };

        let outcome = ingest_summary(config, &tree, ingest_input).await?;

        tracing::info!(
            source_id = %source_id,
            batch = batch_idx,
            summary_id = %outcome.summary_id,
            path = %outcome.content_path,
            sealed = outcome.sealed_ids.len(),
            "[memory_sync:github] batch ingested"
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    // Provider token counts are recorded only when *every* batch reported
    // usage; a mixed run keeps the `len/4` estimate (which covers all
    // batches) rather than a partial real total that would undercount.
    // `estimated_cost_usd` is always populated as the fallback. Issue #3110.
    let any_real_usage = cost.usage_is_real();
    let audit_input_tokens = cost.audit_input_tokens();
    let audit_output_tokens = cost.audit_output_tokens();
    let estimated_cost = cost.estimated_cost();
    let actual_charged_usd = cost.actual_charged_usd();
    // Cost surfaced to the user/logs: real charge when present, else estimate.
    let display_cost = actual_charged_usd.unwrap_or(estimated_cost);

    tracing::info!(
        source_id = %source_id,
        usage_is_real = any_real_usage,
        actual_charge = actual_charged_usd.is_some(),
        input_tokens = audit_input_tokens,
        output_tokens = audit_output_tokens,
        estimated_cost_usd = %format!("{estimated_cost:.4}"),
        actual_charged_usd = ?actual_charged_usd,
        "[memory_sync:github] sync cost accounting"
    );

    append_audit_entry(
        config,
        &SyncAuditEntry {
            timestamp: chrono::Utc::now(),
            source_id: source_id.to_string(),
            source_kind: kind_str.to_string(),
            scope: repo_scope.clone(),
            items_fetched: input_count as u32,
            batches: batch_count as u32,
            input_tokens: audit_input_tokens,
            output_tokens: audit_output_tokens,
            estimated_cost_usd: estimated_cost,
            actual_charged_usd,
            duration_ms,
            success: true,
            error: None,
        },
    );

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Completed,
        Some(kind_str),
        Some(source_id),
        Some(format!(
            "{input_count} items → {batch_count} summary(ies) ({audit_input_tokens} in / {audit_output_tokens} out tokens, ${display_cost:.4})"
        )),
    );

    Ok(SyncOutcome {
        records_ingested: input_count as u32,
        more_pending: false,
        note: Some(format!(
            "{input_count} items → {batch_count} summary(ies) (${display_cost:.4})"
        )),
    })
}

/// Group inputs into batches where each batch's total token count is
/// approximately `budget`. Pairs each batch with its corresponding
/// child labels and basenames for provenance.
fn batch_by_token_budget(
    inputs: &[SummaryInput],
    labels: &[String],
    basenames: &[Option<String>],
    budget: u32,
) -> Vec<(Vec<SummaryInput>, Vec<String>, Vec<Option<String>>)> {
    let mut batches = Vec::new();
    let mut cur_inputs: Vec<SummaryInput> = Vec::new();
    let mut cur_labels: Vec<String> = Vec::new();
    let mut cur_basenames: Vec<Option<String>> = Vec::new();
    let mut cur_tokens: u32 = 0;

    for ((input, label), basename) in inputs.iter().zip(labels.iter()).zip(basenames.iter()) {
        if !cur_inputs.is_empty() && cur_tokens + input.token_count > budget {
            batches.push((
                std::mem::take(&mut cur_inputs),
                std::mem::take(&mut cur_labels),
                std::mem::take(&mut cur_basenames),
            ));
            cur_tokens = 0;
        }
        cur_tokens += input.token_count;
        cur_inputs.push(input.clone());
        cur_labels.push(label.clone());
        cur_basenames.push(basename.clone());
    }

    if !cur_inputs.is_empty() {
        batches.push((cur_inputs, cur_labels, cur_basenames));
    }

    batches
}

fn item_is_high_priority(item_id: &str, content: &SourceContent) -> bool {
    if item_id.starts_with("commit:") {
        return true;
    }
    let state_closed = content
        .metadata
        .get("state")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("closed"))
        .unwrap_or(false);
    let merged = content
        .metadata
        .get("merged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    state_closed || merged
}

fn write_raw_archive(
    raw_source_id: &str,
    item_id: &str,
    updated_at_ms: Option<i64>,
    content: &SourceContent,
    content_root: &std::path::Path,
) {
    let Some((kind, uid)) = github::raw_archive_coords(item_id) else {
        return;
    };
    let created_at_ms = updated_at_ms.unwrap_or(0);
    let raw_item = RawItem {
        uid: &uid,
        created_at_ms,
        markdown: &content.body,
        kind,
    };
    if let Err(e) =
        raw_store::write_raw_items(content_root, raw_source_id, std::slice::from_ref(&raw_item))
    {
        tracing::warn!(
            item_id = %item_id,
            error = %e,
            "[memory_sync:github] raw archive write failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_sources::types::ContentType;

    fn content_with(meta: serde_json::Value) -> SourceContent {
        SourceContent {
            id: "x".into(),
            title: "t".into(),
            body: "b".into(),
            content_type: ContentType::Markdown,
            metadata: meta,
        }
    }

    #[test]
    fn commits_are_always_high_priority() {
        let c = content_with(serde_json::json!({}));
        assert!(item_is_high_priority("commit:abc123", &c));
    }

    #[test]
    fn closed_issue_is_high_priority_open_is_not() {
        let closed = content_with(serde_json::json!({ "state": "closed" }));
        assert!(item_is_high_priority("issue:1", &closed));
        let open = content_with(serde_json::json!({ "state": "open" }));
        assert!(!item_is_high_priority("issue:1", &open));
    }

    #[test]
    fn merged_pr_is_high_priority() {
        let merged = content_with(serde_json::json!({ "state": "open", "merged": true }));
        assert!(item_is_high_priority("pr:7", &merged));
    }

    #[test]
    fn missing_metadata_defaults_to_low_priority() {
        let c = content_with(serde_json::json!({}));
        assert!(!item_is_high_priority("issue:9", &c));
    }

    #[test]
    fn batch_by_budget_groups_correctly() {
        let make = |tokens: u32| SummaryInput {
            id: format!("t{tokens}"),
            content: String::new(),
            token_count: tokens,
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: chrono::Utc::now(),
            time_range_end: chrono::Utc::now(),
            score: 0.5,
        };

        let inputs = vec![make(20_000), make(20_000), make(20_000), make(10_000)];
        let labels: Vec<String> = (0..4).map(|i| format!("l{i}")).collect();
        let basenames: Vec<Option<String>> = (0..4).map(|_| None).collect();
        let batches = batch_by_token_budget(&inputs, &labels, &basenames, 50_000);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].0.len(), 2);
        assert_eq!(batches[0].1.len(), 2);
        assert_eq!(batches[1].0.len(), 2);
    }

    #[test]
    fn batch_empty_input() {
        let batches = batch_by_token_budget(&[], &[], &[], 50_000);
        assert!(batches.is_empty());
    }
}
