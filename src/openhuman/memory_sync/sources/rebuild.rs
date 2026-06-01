//! Rebuild tree from raw archive files on disk.
//!
//! When a sync has ingested raw files but never built summaries (e.g.
//! interrupted sync, or legacy data), this module reads all `.md` files
//! from `raw/<source_slug>/<kind>/`, batches them into ~50k-token
//! groups, summarises each batch, and ingests into the tree via
//! `ingest_summary`.
//!
//! Idempotent: re-running produces new L1 summaries (each has a unique
//! id), so callers should check whether the tree already has L1 nodes
//! before triggering.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree_source::get_or_create_source_tree;
use crate::openhuman::memory_store::content::paths::slugify_source_id;
use crate::openhuman::memory_store::content::raw::raw_source_dir;
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

/// Check whether a source needs tree rebuilding: raw files exist on disk
/// but the tree has no L1+ summaries (max_level == 0).
pub fn needs_rebuild(config: &Config, scope: &str) -> bool {
    let content_root = config.memory_tree_content_root();
    let source_dir = raw_source_dir(&content_root, scope);
    if !source_dir.exists() {
        return false;
    }

    let tree = match get_or_create_source_tree(config, scope) {
        Ok(t) => t,
        Err(_) => return false,
    };

    // Tree has no sealed summaries — raw files are sitting un-summarised.
    if tree.max_level > 0 {
        return false;
    }

    // Check there's actually raw content (not just `_source.md`).
    match collect_raw_files(&source_dir) {
        Ok(files) => !files.is_empty(),
        Err(_) => false,
    }
}

/// Rebuild the tree for a given source scope by reading all raw `.md`
/// files from disk. `scope` is the tree scope (e.g.
/// `"gmail:stevent95-at-gmail-dot-com"`).
///
/// The raw archive lives at `<content_root>/raw/<slugify(scope)>/`.
pub async fn rebuild_tree_from_raw(config: &Config, scope: &str) -> Result<RebuildOutcome> {
    let start = std::time::Instant::now();
    let content_root = config.memory_tree_content_root();
    let source_dir = raw_source_dir(&content_root, scope);

    tracing::info!(
        scope = %scope,
        dir = %source_dir.display(),
        "[memory_sync:rebuild] starting rebuild from raw"
    );

    if !source_dir.exists() {
        anyhow::bail!(
            "raw source directory does not exist: {}",
            source_dir.display()
        );
    }

    // Collect all .md files recursively (skip _source.md).
    let mut files = collect_raw_files(&source_dir)?;
    files.sort(); // chronological order (filename starts with timestamp)

    if files.is_empty() {
        return Ok(RebuildOutcome::default());
    }

    tracing::info!(
        scope = %scope,
        files = files.len(),
        "[memory_sync:rebuild] found raw files"
    );

    // Read all files into SummaryInputs.
    let mut inputs: Vec<SummaryInput> = Vec::with_capacity(files.len());
    let mut basenames: Vec<Option<String>> = Vec::with_capacity(files.len());
    let mut labels: Vec<String> = Vec::with_capacity(files.len());

    for path in &files {
        let body = match std::fs::read_to_string(path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "[memory_sync:rebuild] skipping unreadable file"
                );
                continue;
            }
        };

        let filename = path
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

        // Relative path from content_root for the wikilink (strip .md).
        let rel_path = path
            .strip_prefix(&content_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.strip_suffix(".md").unwrap_or(s).to_string());

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
        basenames.push(rel_path);
    }

    if inputs.is_empty() {
        return Ok(RebuildOutcome {
            files_read: files.len(),
            ..RebuildOutcome::default()
        });
    }

    let tree = get_or_create_source_tree(config, scope)?;

    // Batch and summarise.
    let batches = batch_inputs(&inputs, &labels, &basenames, INPUT_TOKEN_BUDGET);
    let batch_count = batches.len();
    let files_read = inputs.len();

    tracing::info!(
        scope = %scope,
        items = files_read,
        batches = batch_count,
        "[memory_sync:rebuild] summarising"
    );

    // Token/charge accounting across the run. Estimate (`body.len() / 4`) is
    // always summed; provider figures only replace it when every batch
    // reported them (issue #3110). See `RealCostAccumulator`.
    let mut cost = RealCostAccumulator::new();

    for (batch_idx, (batch_inputs, batch_labels, batch_basenames)) in
        batches.into_iter().enumerate()
    {
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

        tracing::info!(
            scope = %scope,
            batch = batch_idx,
            summary_id = %outcome.summary_id,
            "[memory_sync:rebuild] batch ingested"
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
            source_id: format!("rebuild:{scope}"),
            source_kind: "rebuild".to_string(),
            scope: scope.to_string(),
            items_fetched: files_read as u32,
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

    tracing::info!(
        scope = %scope,
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

fn batch_inputs(
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
