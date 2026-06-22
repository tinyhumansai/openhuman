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
use crate::openhuman::memory_sources::readers::SourceReader;
use crate::openhuman::memory_sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};
use crate::openhuman::memory_store::content::raw::{self as raw_store, RawItem};
use crate::openhuman::memory_store::trees::types::TreeKind;
use crate::openhuman::memory_store::trees::types::INPUT_TOKEN_BUDGET;
use crate::openhuman::memory_sync::sources::audit::{
    append_audit_entry, RealCostAccumulator, SyncAuditEntry,
};
use crate::openhuman::memory_sync::traits::{SyncOutcome, SyncPipeline, SyncPipelineKind};
use crate::openhuman::memory_tree::ingest::{ingest_summary, SummaryIngestInput};
use crate::openhuman::memory_tree::summarise::{
    fallback_summary, summarise, SummaryContext, SummaryInput, SummaryOutput,
};
use futures::stream::StreamExt;
use std::path::Path;

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
        Some(source_id),
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
        Some(source_id),
    );

    let content_root = config.memory_tree_content_root();
    let (inputs, child_labels, child_basenames) = read_items_buffered(
        reader.as_ref(),
        &items,
        source,
        config,
        raw_source_id.as_deref(),
        &content_root,
    )
    .await;

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
        Some(source_id),
    );

    // Token/charge accounting across the run. The estimate (`body.len() / 4`
    // heuristic) is always summed; provider-reported figures only replace it
    // when every batch reported them (issue #3110). See `RealCostAccumulator`.
    let mut cost = RealCostAccumulator::new();

    // ── Phase 1: summarise every batch (bounded concurrency) ──
    // The per-batch summarise is an independent LLM round-trip; ingest below
    // is the only ordered, serial part (it writes into the shared source tree).
    // Cloud providers overlap these calls; a local model stays serial. Futures
    // are materialized into a Vec before `buffered` so the higher-ranked
    // closure lifetime stays tied to this frame (avoids the "FnOnce is not
    // general enough" error). `buffered` preserves order so outputs[i] maps to
    // batches[i].
    let concurrency = summarise_concurrency(config.workload_uses_local("memory"));
    tracing::debug!(
        source_id = %source_id,
        tree_id = %tree.id,
        batch_count = batches.len(),
        concurrency,
        "[memory_sync:github] starting concurrent summarise phase"
    );
    let summarise_futs: Vec<_> = batches
        .iter()
        .enumerate()
        .map(|(batch_idx, (batch_inputs, _labels, _basenames))| {
            let config = config;
            let tree_id = &tree.id;
            async move {
                let ctx = SummaryContext {
                    tree_id,
                    tree_kind: TreeKind::Source,
                    target_level: 1,
                    token_budget: 5_000,
                };
                match summarise(config, batch_inputs, &ctx).await {
                    Ok(o) => o,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            batch = batch_idx,
                            "[memory_sync:github] summarise failed, using fallback"
                        );
                        fallback_summary(batch_inputs, ctx.token_budget)
                    }
                }
            }
        })
        .collect();

    let outputs: Vec<SummaryOutput> = futures::stream::iter(summarise_futs)
        .buffered(concurrency)
        .collect()
        .await;
    tracing::debug!(
        source_id = %source_id,
        tree_id = %tree.id,
        outputs = outputs.len(),
        "[memory_sync:github] concurrent summarise phase complete"
    );

    // ── Phase 2: fold cost + ingest in batch order (serial) ──
    for (batch_idx, ((batch_inputs, batch_labels, batch_basenames), output)) in
        batches.into_iter().zip(outputs.into_iter()).enumerate()
    {
        let batch_input_tokens: u64 = batch_inputs.iter().map(|i| i.token_count as u64).sum();

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

        // `child_basenames` holds the raw-archive wikilink paths (rel path
        // with `.md` stripped) — captured BEFORE ingest_summary moves the
        // input. Re-suffix to recover the coverage-gate keys.
        let batch_raw_rel_paths: Vec<String> = ingest_input
            .child_basenames
            .iter()
            .flatten()
            .map(|basename| format!("{basename}.md"))
            .collect();

        let outcome = ingest_summary(config, &tree, ingest_input).await?;

        // Record raw-archive coverage so the incremental reconcile
        // (`memory_sync::sources::rebuild`) knows these files are
        // summarised. Marked only after the summary landed: a crash in
        // between re-summarises the batch (duplicate summary, acceptable)
        // instead of silently losing coverage.
        if !batch_raw_rel_paths.is_empty() {
            if let Err(e) = crate::openhuman::memory_store::chunks::store::mark_raw_paths_ingested(
                config,
                &batch_raw_rel_paths,
            ) {
                tracing::warn!(
                    source_id = %source_id,
                    batch = batch_idx,
                    error = %format!("{e:#}"),
                    "[memory_sync:github] failed to record raw coverage — reconcile may re-summarise this batch"
                );
            }
        }

        tracing::info!(
            source_id = %source_id,
            batch = batch_idx,
            summary_id = %outcome.summary_id,
            path = %outcome.content_path,
            sealed = outcome.sealed_ids.len(),
            covered_raw_files = batch_raw_rel_paths.len(),
            "[memory_sync:github] batch ingested + coverage recorded"
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
            composio_actions_called: 0,
            composio_cost_usd: 0.0,
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
        Some(source_id),
    );

    Ok(SyncOutcome {
        records_ingested: input_count as u32,
        more_pending: false,
        note: Some(format!(
            "{input_count} items → {batch_count} summary(ies) (${display_cost:.4})"
        )),
    })
}

/// Max number of `read_item` round-trips kept in flight at once while reading
/// a transcript's worth of GitHub items. Each read is a GitHub API / `gh` CLI
/// call (issues/PRs always fetch comments — see `read_issue`/`read_pr` — even
/// when the body is served from the list cache), so bounding the fan-out keeps
/// a large repo from opening hundreds of concurrent requests and tripping
/// GitHub's secondary rate limits.
const GITHUB_READ_CONCURRENCY: usize = 8;

/// Max in-flight summarise LLM calls when the memory workload routes to a
/// **cloud** provider. Local models are single-GPU/single-process, so they are
/// driven serially (see [`summarise_concurrency`]).
const GITHUB_SUMMARISE_CONCURRENCY: usize = 4;

/// Effective summarise concurrency given the memory workload routing.
///
/// Cloud providers tolerate (and benefit from) overlapping the per-batch LLM
/// round-trips, so we fan out to [`GITHUB_SUMMARISE_CONCURRENCY`]. A local model
/// is a single inference engine — concurrent calls would just queue (or
/// oversubscribe VRAM), so we keep it strictly serial (`1`), preserving the old
/// one-batch-at-a-time behaviour. Pure so it can be unit-tested.
fn summarise_concurrency(uses_local: bool) -> usize {
    if uses_local {
        1
    } else {
        GITHUB_SUMMARISE_CONCURRENCY
    }
}

/// Read every listed item into the parallel `(inputs, labels, basenames)`
/// vectors used to build the source tree.
///
/// Reads run with **bounded concurrency** (`GITHUB_READ_CONCURRENCY`) instead
/// of one sequential `.await` each: a repo routinely lists 100+ items and every
/// `read_item` is an independent network round-trip, so overlapping their waits
/// is a real wall-time win on this background sync job.
///
/// Order is **preserved** (`buffered`, not `buffer_unordered`): the three
/// returned vectors are positionally aligned and consumed in lockstep by
/// `batch_by_token_budget`, so reordering them would scramble label/basename
/// provenance. A read failure is logged and the item is skipped (it pushes
/// nothing), exactly as the previous sequential `continue` did.
///
/// The per-item futures are collected into a `Vec` before `buffered` for the
/// same higher-ranked-lifetime reason as elsewhere in the codebase: mapping
/// lazily on the stream stores the closure in the polled state and requires it
/// to hold for any lifetime, which fails once the whole sync future is spawned
/// (`Send + 'static`). Collecting builds concrete-lifetime futures up front.
async fn read_items_buffered(
    reader: &dyn SourceReader,
    items: &[SourceItem],
    source: &MemorySourceEntry,
    config: &Config,
    raw_source_id: Option<&str>,
    content_root: &Path,
) -> (Vec<SummaryInput>, Vec<String>, Vec<Option<String>>) {
    let total = items.len();
    let kind_str = source.kind.as_str();
    let source_id = source.id.as_str();

    let item_futs: Vec<_> = items
        .iter()
        .map(|item| async move {
            let content = match reader.read_item(source, &item.id, config).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        item_id = %item.id,
                        error = %e,
                        "[memory_sync:github] skipping item — read failed"
                    );
                    return None;
                }
            };

            if let Some(raw_sid) = raw_source_id {
                write_raw_archive(
                    raw_sid,
                    &item.id,
                    item.updated_at_ms,
                    &content,
                    content_root,
                );
            }

            // Compute the raw archive relative path for the wikilink.
            let raw_path = raw_source_id.and_then(|raw_sid| {
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

            Some((
                SummaryInput {
                    id: item.id.clone(),
                    content: content.body,
                    token_count,
                    entities: Vec::new(),
                    topics: vec!["memory_sources".to_string(), kind_str.to_string()],
                    time_range_start: ts,
                    time_range_end: ts,
                    score: if priority { 0.8 } else { 0.5 },
                },
                item.id.clone(),
                raw_path,
            ))
        })
        .collect();

    let mut inputs: Vec<SummaryInput> = Vec::with_capacity(total);
    let mut child_labels: Vec<String> = Vec::with_capacity(total);
    let mut child_basenames: Vec<Option<String>> = Vec::with_capacity(total);

    let mut stream = futures::stream::iter(item_futs).buffered(GITHUB_READ_CONCURRENCY);
    let mut processed = 0usize;
    while let Some(result) = stream.next().await {
        if let Some((input, label, raw_path)) = result {
            inputs.push(input);
            child_labels.push(label);
            child_basenames.push(raw_path);
        }
        processed += 1;
        if processed % 100 == 0 || processed == total {
            emit_sync_stage(
                MemorySyncTrigger::Manual,
                MemorySyncStage::Ingesting,
                Some(kind_str),
                Some(source_id),
                Some(format!("{processed}/{total} read")),
                Some(source_id),
            );
        }
    }

    (inputs, child_labels, child_basenames)
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

    #[test]
    fn summarise_concurrency_serial_for_local_model() {
        // A local model is a single inference engine — must stay serial.
        assert_eq!(summarise_concurrency(true), 1);
    }

    #[test]
    fn summarise_concurrency_fans_out_for_cloud_model() {
        assert_eq!(summarise_concurrency(false), GITHUB_SUMMARISE_CONCURRENCY);
        assert!(GITHUB_SUMMARISE_CONCURRENCY > 1, "cloud path must overlap");
    }

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

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// `SourceReader` double that records the high-water mark of concurrent
    /// `read_item` calls, proving `read_items_buffered` overlaps reads while
    /// staying within `GITHUB_READ_CONCURRENCY`. Fails reads whose id contains
    /// "fail" so the skip path is exercised too.
    struct CountingReader {
        in_flight: AtomicUsize,
        peak_in_flight: AtomicUsize,
        calls: AtomicUsize,
    }

    impl CountingReader {
        fn new() -> Self {
            Self {
                in_flight: AtomicUsize::new(0),
                peak_in_flight: AtomicUsize::new(0),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl SourceReader for CountingReader {
        fn kind(&self) -> SourceKind {
            SourceKind::GithubRepo
        }

        async fn list_items(
            &self,
            _source: &MemorySourceEntry,
            _config: &Config,
        ) -> Result<Vec<SourceItem>, String> {
            Ok(Vec::new())
        }

        async fn read_item(
            &self,
            _source: &MemorySourceEntry,
            item_id: &str,
            _config: &Config,
        ) -> Result<SourceContent, String> {
            // Track concurrent entry, then yield repeatedly *before* returning
            // so sibling reads genuinely overlap in time — otherwise a read
            // that completes on first poll would never reveal concurrency.
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak_in_flight.fetch_max(now, Ordering::SeqCst);
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.in_flight.fetch_sub(1, Ordering::SeqCst);

            if item_id.contains("fail") {
                return Err(format!("simulated read failure for {item_id}"));
            }
            Ok(SourceContent {
                id: item_id.to_string(),
                title: format!("title {item_id}"),
                body: format!("body for {item_id}"),
                content_type: ContentType::Markdown,
                metadata: serde_json::json!({}),
            })
        }
    }

    fn github_source() -> MemorySourceEntry {
        MemorySourceEntry {
            id: "src_gh".into(),
            kind: SourceKind::GithubRepo,
            label: "gh".into(),
            enabled: true,
            toolkit: None,
            connection_id: None,
            path: None,
            glob: None,
            url: Some("https://github.com/owner/repo".into()),
            branch: None,
            paths: Vec::new(),
            max_commits: None,
            max_issues: None,
            max_prs: None,
            query: None,
            since_days: None,
            max_items: None,
            selector: None,
            max_tokens_per_sync: None,
            max_cost_per_sync_usd: None,
            sync_depth_days: None,
        }
    }

    fn issue_items(n: usize) -> Vec<SourceItem> {
        (0..n)
            .map(|i| SourceItem {
                id: format!("issue:{i}"),
                title: format!("t{i}"),
                updated_at_ms: Some(1_700_000_000_000 + i as i64),
            })
            .collect()
    }

    #[tokio::test]
    async fn read_items_buffered_overlaps_and_preserves_order() {
        // 20 items exceeds GITHUB_READ_CONCURRENCY (8): an unbounded fan-out
        // would push >8 reads in flight (bound assertion fails), while a fully
        // sequential read would never exceed peak 1 (overlap assertion fails).
        let reader = CountingReader::new();
        let source = github_source();
        let config = Config::default();
        let items = issue_items(20);

        let (inputs, labels, basenames) = read_items_buffered(
            &reader,
            &items,
            &source,
            &config,
            None,
            Path::new("/tmp/openhuman-test-unused"),
        )
        .await;

        assert_eq!(inputs.len(), 20);
        assert_eq!(basenames.len(), 20);
        assert!(
            basenames.iter().all(Option::is_none),
            "no raw_source_id → no basenames"
        );

        // Order preserved: labels and input ids must match input order exactly.
        let want: Vec<String> = (0..20).map(|i| format!("issue:{i}")).collect();
        assert_eq!(labels, want, "buffered must preserve input order");
        assert_eq!(
            inputs.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            want
        );

        assert_eq!(reader.calls.load(Ordering::SeqCst), 20);
        let peak = reader.peak_in_flight.load(Ordering::SeqCst);
        assert!(
            peak <= GITHUB_READ_CONCURRENCY,
            "read concurrency must stay bounded at {GITHUB_READ_CONCURRENCY}, observed peak {peak}"
        );
        assert!(
            peak >= 2,
            "reads must actually overlap (else the bound is meaningless), observed peak {peak}"
        );
    }

    #[tokio::test]
    async fn read_items_buffered_skips_failed_reads_and_keeps_order() {
        let reader = CountingReader::new();
        let source = github_source();
        let config = Config::default();
        // Middle item fails its read; it must be skipped without breaking the
        // positional alignment of the surviving items.
        let items = vec![
            SourceItem {
                id: "issue:0".into(),
                title: "a".into(),
                updated_at_ms: Some(1),
            },
            SourceItem {
                id: "issue:fail".into(),
                title: "b".into(),
                updated_at_ms: Some(2),
            },
            SourceItem {
                id: "issue:2".into(),
                title: "c".into(),
                updated_at_ms: Some(3),
            },
        ];

        let (inputs, labels, _basenames) =
            read_items_buffered(&reader, &items, &source, &config, None, Path::new("/tmp/x")).await;

        assert_eq!(inputs.len(), 2, "failed item skipped");
        assert_eq!(labels, vec!["issue:0".to_string(), "issue:2".to_string()]);
    }
}
