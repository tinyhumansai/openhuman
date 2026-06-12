//! Reconcile raw archive files on disk into the memory tree.
//!
//! Raw archive files are written eagerly at fetch time (one `.md` per
//! upstream item under `raw/<source_slug>/<kind>/`), but summaries only
//! land at the end of a sync's batch loop — so an interrupted sync, a
//! crash mid-summarise, or legacy data leaves raw files on disk with no
//! tree coverage. This module closes that gap **incrementally**:
//!
//! 1. Every successful summary-batch ingest records its raw files in the
//!    `mem_tree_ingested_sources` coverage gate
//!    (`source_kind = "raw_file"`, `source_id = <rel path>`).
//! 2. [`raw_coverage`] lists the on-disk files NOT in the gate. For
//!    scopes whose summaries predate the gate (legacy data), it first
//!    backfills the gate from the existing L1 summaries' child labels so
//!    already-covered files are never re-summarised.
//! 3. [`rebuild_tree_from_raw`] reads only the pending files, batches
//!    them into ~50k-token groups, summarises each batch, ingests via
//!    `ingest_summary`, and marks the batch's files covered.
//!
//! Idempotent: re-running with full coverage is a no-op; a run that dies
//! mid-way resumes from the first unmarked batch.
//!
//! ## Tree scope vs archive id
//!
//! A source has TWO identifiers that slugify to *different* directories:
//! the tree scope (e.g. `github:owner/repo` → tree registry key) and the
//! raw-archive source id (e.g. `github.com/owner/repo` →
//! `raw/github-com-owner-repo/`). Callers must pass both — deriving the
//! raw dir from the tree scope silently scans the wrong (usually empty)
//! directory, which is exactly the bug that let thousands of GitHub raw
//! files sit unreconciled.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree_source::get_or_create_source_tree;
use crate::openhuman::memory_store::chunks::store::{
    count_raw_paths_ingested_with_prefix, filter_raw_paths_not_ingested,
    list_chunk_raw_ref_paths_with_prefix, mark_raw_paths_ingested,
};
use crate::openhuman::memory_store::content::paths::slugify_source_id;
use crate::openhuman::memory_store::content::raw::{raw_source_dir, sanitize_uid};
use crate::openhuman::memory_store::trees::store::list_summaries_at_level;
use crate::openhuman::memory_store::trees::types::{TreeKind, INPUT_TOKEN_BUDGET};
use crate::openhuman::memory_sync::sources::audit::{
    append_audit_entry, RealCostAccumulator, SyncAuditEntry,
};
use crate::openhuman::memory_tree::ingest::{ingest_summary, SummaryIngestInput};
use crate::openhuman::memory_tree::summarise::{
    fallback_summary, summarise, SummaryContext, SummaryInput,
};

/// Outcome of a rebuild operation.
#[derive(Clone, Debug, Default)]
pub struct RebuildOutcome {
    pub files_read: usize,
    pub batches: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    /// Real amount billed by the backend in USD when the provider reported
    /// usage for the run; `None` when it fell back to the estimate. Issue
    /// #3110. Prefer this over `estimated_cost_usd` when `Some`.
    pub actual_charged_usd: Option<f64>,
}

/// One raw archive file pending tree coverage.
#[derive(Clone, Debug)]
pub struct RawFileRef {
    /// Absolute path on disk.
    pub abs: PathBuf,
    /// Forward-slash relative path under `<content_root>/`, with `.md` —
    /// the canonical coverage-gate key (matches `raw::raw_rel_path`).
    pub rel: String,
}

/// Coverage report for one source's raw archive.
#[derive(Clone, Debug, Default)]
pub struct RawCoverage {
    /// Total `.md` files on disk (excluding `_source.md` etc.).
    pub total: usize,
    /// Files recorded in the coverage gate.
    pub covered: usize,
    /// Files on disk with no coverage record, sorted chronologically
    /// (filenames start with the item timestamp).
    pub pending: Vec<RawFileRef>,
}

/// Compute raw-archive coverage for a source.
///
/// `tree_scope` keys the source tree (e.g. `"github:owner/repo"`,
/// `"gmail:user-at-gmail-dot-com"`); `archive_source_id` keys the raw
/// archive directory (e.g. `"github.com/owner/repo"` — pass the tree
/// scope again when the source writes its archive under the same id,
/// as gmail does).
///
/// On first call for a scope with existing L1 summaries but an empty
/// gate (legacy data from before coverage tracking), the gate is
/// backfilled from the summaries' child labels so previously-summarised
/// files don't get re-ingested.
pub fn raw_coverage(
    config: &Config,
    tree_scope: &str,
    archive_source_id: &str,
) -> Result<RawCoverage> {
    let content_root = config.memory_tree_content_root();
    let source_dir = raw_source_dir(&content_root, archive_source_id);
    if !source_dir.exists() {
        return Ok(RawCoverage::default());
    }

    let mut files = collect_raw_files(&source_dir)?;
    files.sort();
    if files.is_empty() {
        return Ok(RawCoverage::default());
    }

    let refs: Vec<RawFileRef> = files
        .into_iter()
        .filter_map(|abs| {
            let rel = abs
                .strip_prefix(&content_root)
                .ok()?
                .to_str()?
                .replace(std::path::MAIN_SEPARATOR, "/");
            Some(RawFileRef { abs, rel })
        })
        .collect();
    let total = refs.len();

    // Legacy backfill: gate empty for this archive but the tree already
    // has L1 summaries → mark the files those summaries cover.
    let rel_prefix = format!("raw/{}/", slugify_source_id(archive_source_id));
    if count_raw_paths_ingested_with_prefix(config, &rel_prefix)? == 0 {
        let backfilled = backfill_coverage_from_summaries(config, tree_scope, &refs)?;
        if backfilled > 0 {
            tracing::info!(
                tree_scope = %tree_scope,
                archive = %archive_source_id,
                backfilled = backfilled,
                "[memory_sync:rebuild] backfilled coverage gate from existing L1 summaries"
            );
        }
    }

    let rel_paths: Vec<String> = refs.iter().map(|r| r.rel.clone()).collect();
    let mut pending_rels: HashSet<String> = filter_raw_paths_not_ingested(config, &rel_paths)?
        .into_iter()
        .collect();

    // A raw file referenced by a persisted chunk (`raw_refs_json`) is
    // already in the tree via the chunk pipeline — gmail mirrors every
    // email to raw/ AND ingests it as a chunk. Those files are covered;
    // re-summarising them through the rebuild path would duplicate
    // content (and burn LLM batches) for sources that never use the
    // summarise-direct path at all.
    let chunk_covered = list_chunk_raw_ref_paths_with_prefix(config, &rel_prefix)?;
    if !chunk_covered.is_empty() {
        pending_rels.retain(|rel| !chunk_covered.contains(rel));
    }

    let pending: Vec<RawFileRef> = refs
        .into_iter()
        .filter(|r| pending_rels.contains(&r.rel))
        .collect();

    tracing::debug!(
        tree_scope = %tree_scope,
        archive = %archive_source_id,
        total = total,
        pending = pending.len(),
        "[memory_sync:rebuild] raw coverage computed"
    );

    Ok(RawCoverage {
        total,
        covered: total - pending.len(),
        pending,
    })
}

/// Check whether a source has raw files on disk that the tree does not
/// cover yet. Coverage-based: a partially-covered scope (interrupted
/// sync) returns `true` even when the tree already has L1 summaries.
pub fn needs_rebuild(config: &Config, tree_scope: &str, archive_source_id: &str) -> bool {
    match raw_coverage(config, tree_scope, archive_source_id) {
        Ok(cov) => !cov.pending.is_empty(),
        Err(e) => {
            tracing::warn!(
                tree_scope = %tree_scope,
                archive = %archive_source_id,
                error = %format!("{e:#}"),
                "[memory_sync:rebuild] coverage check failed — skipping rebuild"
            );
            false
        }
    }
}

/// Mark a label set covered in the gate from existing L1 summaries.
///
/// Sync-written summaries label children as `commit:<sha>` / `issue:<n>` /
/// `pr:<n>`; rebuild-written summaries label them with the raw file stem
/// (`<ts>_<uid>`). Both shapes are matched against the on-disk files.
/// Returns the number of files marked covered.
fn backfill_coverage_from_summaries(
    config: &Config,
    tree_scope: &str,
    files: &[RawFileRef],
) -> Result<u64> {
    let tree = get_or_create_source_tree(config, tree_scope)
        .map_err(|e| anyhow::anyhow!("get_or_create_source_tree: {e:#}"))?;
    if tree.max_level == 0 {
        return Ok(0);
    }

    let summaries = list_summaries_at_level(config, &tree.id, 1)?;
    // (kind_dir, sanitised uid) pairs from `<prefix>:<uid>` labels.
    let mut kind_uids: HashSet<(&'static str, String)> = HashSet::new();
    // Full file stems from rebuild-written labels.
    let mut stems: HashSet<String> = HashSet::new();
    for summary in &summaries {
        for label in &summary.child_ids {
            if let Some(uid) = label.strip_prefix("commit:") {
                kind_uids.insert(("commits", sanitize_uid(uid)));
            } else if let Some(uid) = label.strip_prefix("issue:") {
                kind_uids.insert(("issues", sanitize_uid(uid)));
            } else if let Some(uid) = label.strip_prefix("pr:") {
                kind_uids.insert(("prs", sanitize_uid(uid)));
            } else {
                stems.insert(label.clone());
            }
        }
    }
    if kind_uids.is_empty() && stems.is_empty() {
        return Ok(0);
    }

    let mut covered: Vec<String> = Vec::new();
    for f in files {
        // rel = raw/<slug>/<kind>/<ts>_<uid>.md
        let mut parts = f.rel.rsplitn(2, '/');
        let filename = parts.next().unwrap_or_default();
        let kind_dir = parts
            .next()
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or_default();
        let stem = filename.strip_suffix(".md").unwrap_or(filename);
        let uid = stem.split_once('_').map(|(_, u)| u).unwrap_or(stem);

        let label_match =
            stems.contains(stem) || kind_uids.iter().any(|(k, u)| *k == kind_dir && u == uid);
        if label_match {
            covered.push(f.rel.clone());
        }
    }

    mark_raw_paths_ingested(config, &covered)
}

/// Summarise + ingest every **pending** raw file for a source. `tree_scope`
/// and `archive_source_id` as in [`raw_coverage`].
///
/// Each batch's files are marked covered immediately after that batch's
/// summary lands, so an interrupted run resumes where it left off.
pub async fn rebuild_tree_from_raw(
    config: &Config,
    tree_scope: &str,
    archive_source_id: &str,
) -> Result<RebuildOutcome> {
    let start = std::time::Instant::now();
    let content_root = config.memory_tree_content_root();

    let coverage = raw_coverage(config, tree_scope, archive_source_id)?;
    tracing::info!(
        tree_scope = %tree_scope,
        archive = %archive_source_id,
        total = coverage.total,
        covered = coverage.covered,
        pending = coverage.pending.len(),
        "[memory_sync:rebuild] starting incremental rebuild"
    );

    if coverage.pending.is_empty() {
        return Ok(RebuildOutcome::default());
    }
    let files = coverage.pending;

    // Read pending files into SummaryInputs.
    let mut inputs: Vec<SummaryInput> = Vec::with_capacity(files.len());
    let mut basenames: Vec<Option<String>> = Vec::with_capacity(files.len());
    let mut labels: Vec<String> = Vec::with_capacity(files.len());
    let mut rel_paths: Vec<String> = Vec::with_capacity(files.len());

    for file in &files {
        let body = match std::fs::read_to_string(&file.abs) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    path = %file.abs.display(),
                    error = %e,
                    "[memory_sync:rebuild] skipping unreadable file"
                );
                continue;
            }
        };

        let filename = file
            .abs
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        // Parse timestamp from filename: <ts_ms>_<uid>.md
        let ts_ms = filename
            .split('_')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let ts = chrono::DateTime::from_timestamp_millis(ts_ms).unwrap_or_else(chrono::Utc::now);

        let token_count = (body.len() / 4).max(1) as u32;

        // Wikilink basename: rel path with `.md` stripped.
        let wikilink = file
            .rel
            .strip_suffix(".md")
            .unwrap_or(&file.rel)
            .to_string();

        inputs.push(SummaryInput {
            id: filename.to_string(),
            content: body,
            token_count,
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
        });
        labels.push(filename.to_string());
        basenames.push(Some(wikilink));
        rel_paths.push(file.rel.clone());
    }

    if inputs.is_empty() {
        return Ok(RebuildOutcome {
            files_read: files.len(),
            ..RebuildOutcome::default()
        });
    }

    let tree = get_or_create_source_tree(config, tree_scope)
        .map_err(|e| anyhow::anyhow!("get_or_create_source_tree: {e:#}"))?;

    // Batch and summarise.
    let batches = batch_inputs(&inputs, &labels, &basenames, &rel_paths, INPUT_TOKEN_BUDGET);
    let batch_count = batches.len();
    let files_read = inputs.len();

    tracing::info!(
        tree_scope = %tree_scope,
        items = files_read,
        batches = batch_count,
        "[memory_sync:rebuild] summarising"
    );

    // Token/charge accounting across the run. Estimate (`body.len() / 4`) is
    // always summed; provider figures only replace it when every batch
    // reported them (issue #3110). See `RealCostAccumulator`.
    let mut cost = RealCostAccumulator::new();

    for (batch_idx, batch) in batches.into_iter().enumerate() {
        let RebuildBatch {
            inputs: batch_inputs,
            labels: batch_labels,
            basenames: batch_basenames,
            rel_paths: batch_rel_paths,
        } = batch;
        let batch_in_tokens: u64 = batch_inputs.iter().map(|i| i.token_count as u64).sum();

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
                    "[memory_sync:rebuild] summarise failed, using fallback"
                );
                fallback_summary(&batch_inputs, ctx.token_budget)
            }
        };

        cost.add_batch(
            batch_in_tokens,
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

        let ingest_input = SummaryIngestInput {
            content: output.content,
            token_count: output.token_count,
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: time_start,
            time_range_end: time_end,
            score: 0.5,
            child_labels: batch_labels,
            child_basenames: batch_basenames,
        };

        let outcome = ingest_summary(config, &tree, ingest_input).await?;

        // Mark coverage ONLY after the summary landed — a crash between
        // ingest and mark re-summarises this batch (duplicate summary,
        // acceptable) instead of silently dropping coverage (data loss).
        if let Err(e) = mark_raw_paths_ingested(config, &batch_rel_paths) {
            tracing::warn!(
                batch = batch_idx,
                error = %format!("{e:#}"),
                "[memory_sync:rebuild] failed to record raw coverage — batch may re-summarise"
            );
        }

        tracing::info!(
            tree_scope = %tree_scope,
            batch = batch_idx,
            summary_id = %outcome.summary_id,
            files = batch_rel_paths.len(),
            "[memory_sync:rebuild] batch ingested + coverage recorded"
        );
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    // Provider figures are recorded only when *every* batch reported them; a
    // mixed run keeps the `len/4` estimate (which covers all batches) rather
    // than a partial real total that would undercount. `estimated_cost_usd`
    // is always populated as the fallback. Issue #3110.
    let any_real_usage = cost.usage_is_real();
    let audit_input_tokens = cost.audit_input_tokens();
    let audit_output_tokens = cost.audit_output_tokens();
    let estimated_cost = cost.estimated_cost();
    let actual_charged_usd = cost.actual_charged_usd();
    let display_cost = actual_charged_usd.unwrap_or(estimated_cost);

    append_audit_entry(
        config,
        &SyncAuditEntry {
            timestamp: chrono::Utc::now(),
            source_id: format!("rebuild:{tree_scope}"),
            source_kind: "rebuild".to_string(),
            scope: tree_scope.to_string(),
            items_fetched: files_read as u32,
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

    tracing::info!(
        tree_scope = %tree_scope,
        files = files_read,
        batches = batch_count,
        usage_is_real = any_real_usage,
        actual_charge = actual_charged_usd.is_some(),
        input_tokens = audit_input_tokens,
        output_tokens = audit_output_tokens,
        estimated_cost_usd = %format!("{estimated_cost:.4}"),
        actual_charged_usd = ?actual_charged_usd,
        display_cost_usd = %format!("{display_cost:.4}"),
        duration_ms = duration_ms,
        "[memory_sync:rebuild] complete"
    );

    Ok(RebuildOutcome {
        files_read,
        batches: batch_count,
        input_tokens: audit_input_tokens,
        output_tokens: audit_output_tokens,
        estimated_cost_usd: estimated_cost,
        actual_charged_usd,
    })
}

/// Collect all `.md` files recursively under `dir`, skipping `_source.md`.
fn collect_raw_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_recursive(dir, &mut files)?;
    Ok(files)
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| anyhow::anyhow!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            if fname.starts_with('_') {
                continue;
            }
            out.push(path);
        }
    }
    Ok(())
}

/// One summarise-and-ingest batch with its parallel provenance vectors.
struct RebuildBatch {
    inputs: Vec<SummaryInput>,
    labels: Vec<String>,
    basenames: Vec<Option<String>>,
    rel_paths: Vec<String>,
}

fn batch_inputs(
    inputs: &[SummaryInput],
    labels: &[String],
    basenames: &[Option<String>],
    rel_paths: &[String],
    budget: u32,
) -> Vec<RebuildBatch> {
    let mut batches: Vec<RebuildBatch> = Vec::new();
    let mut cur = RebuildBatch {
        inputs: Vec::new(),
        labels: Vec::new(),
        basenames: Vec::new(),
        rel_paths: Vec::new(),
    };
    let mut cur_tokens: u32 = 0;

    for i in 0..inputs.len() {
        if !cur.inputs.is_empty() && cur_tokens + inputs[i].token_count > budget {
            batches.push(std::mem::replace(
                &mut cur,
                RebuildBatch {
                    inputs: Vec::new(),
                    labels: Vec::new(),
                    basenames: Vec::new(),
                    rel_paths: Vec::new(),
                },
            ));
            cur_tokens = 0;
        }
        cur_tokens += inputs[i].token_count;
        cur.inputs.push(inputs[i].clone());
        cur.labels.push(labels[i].clone());
        cur.basenames.push(basenames[i].clone());
        cur.rel_paths.push(rel_paths[i].clone());
    }

    if !cur.inputs.is_empty() {
        batches.push(cur);
    }

    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        cfg
    }

    fn write_raw(cfg: &Config, archive_id: &str, kind: &str, name: &str, body: &str) {
        let dir = cfg
            .memory_tree_content_root()
            .join("raw")
            .join(slugify_source_id(archive_id))
            .join(kind);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn collect_raw_files_skips_underscore_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let emails = tmp.path().join("emails");
        std::fs::create_dir_all(&emails).unwrap();
        std::fs::write(emails.join("1000_abc.md"), "body").unwrap();
        std::fs::write(emails.join("2000_def.md"), "body2").unwrap();
        std::fs::write(emails.join("_source.md"), "meta").unwrap();

        let files = collect_raw_files(tmp.path()).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files
            .iter()
            .all(|f| !f.to_str().unwrap().contains("_source")));
    }

    #[test]
    fn collect_raw_files_recurses_subdirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = tmp.path().join("commits");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("100_sha.md"), "commit").unwrap();
        std::fs::write(tmp.path().join("top.md"), "top").unwrap();

        let files = collect_raw_files(tmp.path()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn batch_inputs_carries_rel_paths_alongside_provenance() {
        let make = |tokens: u32, id: &str| SummaryInput {
            id: id.to_string(),
            content: String::new(),
            token_count: tokens,
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: chrono::Utc::now(),
            time_range_end: chrono::Utc::now(),
            score: 0.5,
        };
        let inputs = vec![make(30_000, "a"), make(30_000, "b"), make(30_000, "c")];
        let labels: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let basenames: Vec<Option<String>> = vec![None, None, None];
        let rels: Vec<String> = vec![
            "raw/x/a.md".into(),
            "raw/x/b.md".into(),
            "raw/x/c.md".into(),
        ];

        let batches = batch_inputs(&inputs, &labels, &basenames, &rels, 50_000);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].rel_paths, vec!["raw/x/a.md".to_string()]);
        assert_eq!(batches[2].labels, vec!["c".to_string()]);
    }

    #[test]
    fn raw_coverage_reports_all_pending_for_untracked_archive() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        write_raw(&cfg, "github.com/o/r", "commits", "100_aaa.md", "a");
        write_raw(&cfg, "github.com/o/r", "issues", "200_7.md", "b");

        let cov = raw_coverage(&cfg, "github:o/r", "github.com/o/r").unwrap();
        assert_eq!(cov.total, 2);
        assert_eq!(cov.covered, 0);
        assert_eq!(cov.pending.len(), 2);
        assert!(cov.pending[0].rel.starts_with("raw/github-com-o-r/"));
        assert!(needs_rebuild(&cfg, "github:o/r", "github.com/o/r"));
    }

    #[test]
    fn raw_coverage_empty_when_archive_dir_missing() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let cov = raw_coverage(&cfg, "github:o/r", "github.com/o/r").unwrap();
        assert_eq!(cov.total, 0);
        assert!(cov.pending.is_empty());
        assert!(!needs_rebuild(&cfg, "github:o/r", "github.com/o/r"));
    }

    #[test]
    fn marked_files_drop_out_of_pending() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        write_raw(&cfg, "github.com/o/r", "commits", "100_aaa.md", "a");
        write_raw(&cfg, "github.com/o/r", "commits", "200_bbb.md", "b");

        mark_raw_paths_ingested(&cfg, &["raw/github-com-o-r/commits/100_aaa.md".to_string()])
            .unwrap();

        let cov = raw_coverage(&cfg, "github:o/r", "github.com/o/r").unwrap();
        assert_eq!(cov.total, 2);
        assert_eq!(cov.covered, 1);
        assert_eq!(cov.pending.len(), 1);
        assert!(cov.pending[0].rel.ends_with("200_bbb.md"));
    }

    #[tokio::test]
    async fn backfill_marks_files_covered_by_existing_summaries() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let scope = "github:o/r";
        let archive = "github.com/o/r";

        // Two raw files on disk; an existing L1 summary covers only the
        // commit (sync-written `commit:<sha>` label shape).
        write_raw(&cfg, archive, "commits", "100_abc123.md", "commit body");
        write_raw(&cfg, archive, "issues", "200_42.md", "issue body");

        let tree = get_or_create_source_tree(&cfg, scope).unwrap();
        let input = SummaryIngestInput {
            content: "summary of one commit".to_string(),
            token_count: 10,
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: chrono::Utc::now(),
            time_range_end: chrono::Utc::now(),
            score: 0.5,
            child_labels: vec!["commit:abc123".to_string()],
            child_basenames: Vec::new(),
        };
        ingest_summary(&cfg, &tree, input).await.unwrap();

        let cov = raw_coverage(&cfg, scope, archive).unwrap();
        assert_eq!(cov.total, 2);
        assert_eq!(cov.covered, 1, "commit file backfilled as covered");
        assert_eq!(cov.pending.len(), 1);
        assert!(cov.pending[0].rel.ends_with("200_42.md"));
    }

    #[test]
    fn chunk_raw_refs_count_as_covered() {
        use crate::openhuman::memory_store::chunks::store::{
            set_chunk_raw_refs, upsert_chunks, RawRef,
        };
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind as ChunkSourceKind, SourceRef,
        };

        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let scope = "gmail:user-at-example-dot-com";

        // Two raw email files; one is referenced by a persisted chunk
        // (the gmail pipeline shape), the other is orphaned.
        write_raw(&cfg, scope, "emails", "1000_msg-a.md", "email a");
        write_raw(&cfg, scope, "emails", "2000_msg-b.md", "email b");

        let now = chrono::Utc::now();
        let c = Chunk {
            id: chunk_id(ChunkSourceKind::Email, scope, 0, "email a"),
            content: "email a".into(),
            metadata: Metadata {
                source_kind: ChunkSourceKind::Email,
                source_id: scope.into(),
                owner: "user".into(),
                timestamp: now,
                time_range: (now, now),
                tags: vec![],
                source_ref: Some(SourceRef::new("gmail://msg-a")),
                path_scope: None,
            },
            token_count: 2,
            seq_in_source: 0,
            created_at: now,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        set_chunk_raw_refs(
            &cfg,
            &c.id,
            &[RawRef {
                path: "raw/gmail-user-at-example-dot-com/emails/1000_msg-a.md".into(),
                start: 0,
                end: None,
            }],
        )
        .unwrap();

        let cov = raw_coverage(&cfg, scope, scope).unwrap();
        assert_eq!(cov.total, 2);
        assert_eq!(cov.covered, 1, "chunk-referenced file is covered");
        assert_eq!(cov.pending.len(), 1);
        assert!(cov.pending[0].rel.ends_with("2000_msg-b.md"));
    }

    #[tokio::test]
    async fn rebuild_processes_only_pending_and_records_coverage() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let scope = "github:o/r2";
        let archive = "github.com/o/r2";

        write_raw(&cfg, archive, "commits", "100_aaa.md", "first commit body");
        write_raw(&cfg, archive, "commits", "200_bbb.md", "second commit body");
        mark_raw_paths_ingested(
            &cfg,
            &["raw/github-com-o-r2/commits/100_aaa.md".to_string()],
        )
        .unwrap();

        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("rebuilt summary content"));
        let outcome = test_override::with_provider(provider, async {
            rebuild_tree_from_raw(&cfg, scope, archive).await.unwrap()
        })
        .await;

        assert_eq!(outcome.files_read, 1, "only the pending file is read");
        assert_eq!(outcome.batches, 1);

        // Idempotent: a second run finds nothing pending.
        let cov = raw_coverage(&cfg, scope, archive).unwrap();
        assert!(cov.pending.is_empty(), "all files covered after rebuild");
        assert!(!needs_rebuild(&cfg, scope, archive));
    }
}
