//! Agent-facing `memory_diff` tool.
//!
//! Lets agents query what changed in memory sources since the last sync
//! or a named checkpoint, formatted as concise markdown.

use async_trait::async_trait;
use log::debug;
use serde_json::{json, Value};

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops;
use super::types::*;

pub struct MemoryDiffTool;

#[async_trait]
impl Tool for MemoryDiffTool {
    fn name(&self) -> &str {
        "memory_diff"
    }

    fn description(&self) -> &str {
        "Check what changed in memory sources since the last sync or a named checkpoint. \
         Returns a structured summary of added, removed, and modified items across one or all sources."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source_id": {
                    "type": "string",
                    "description": "Memory source id. If omitted and checkpoint_id is also omitted, \
                                    lists available sources with snapshot counts."
                },
                "checkpoint_id": {
                    "type": "string",
                    "description": "Checkpoint id to diff against. If provided, computes cross-source \
                                    diff since that checkpoint."
                },
                "include_text_diff": {
                    "type": "boolean",
                    "description": "If true, include line-level text diffs for modified items (truncated).",
                    "default": false
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let source_id = args.get("source_id").and_then(|v| v.as_str());
        let checkpoint_id = args.get("checkpoint_id").and_then(|v| v.as_str());
        let include_text_diff = args
            .get("include_text_diff")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        debug!(
            "[memory_diff][tool] execute source_id={:?} checkpoint_id={:?} include_text_diff={}",
            source_id, checkpoint_id, include_text_diff
        );

        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        if let Some(ckpt_id) = checkpoint_id {
            debug!("[memory_diff][tool] branch=checkpoint_diff checkpoint_id={ckpt_id}");
            let diff = ops::diff_since_checkpoint(ckpt_id, &config, include_text_diff)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            let md = format_cross_source_diff(&diff);
            return Ok(ToolResult::success(md));
        }

        if let Some(sid) = source_id {
            debug!("[memory_diff][tool] branch=source_diff source_id={sid}");
            let source = crate::openhuman::memory_sources::get_source(sid)
                .await
                .map_err(|e| anyhow::anyhow!(e))?
                .ok_or_else(|| anyhow::anyhow!("source not found: {sid}"))?;

            let diff = ops::diff_since_last(&source, &config, include_text_diff)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            let md = format_diff_result(&diff);
            return Ok(ToolResult::success(md));
        }

        debug!("[memory_diff][tool] branch=list_sources");
        // No source_id or checkpoint_id: list sources with snapshot counts
        let sources = crate::openhuman::memory_sources::list_sources()
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let workspace_dir = config.workspace_dir.clone();
        let source_ids: Vec<(String, String, String)> = sources
            .iter()
            .filter(|s| s.enabled)
            .map(|s| (s.id.clone(), s.label.clone(), s.kind.as_str().to_string()))
            .collect();

        let counts: Vec<(String, String, String, usize)> = tokio::task::spawn_blocking(move || {
            super::store::with_connection(&workspace_dir, |conn| {
                let mut out = Vec::new();
                for (sid, label, kind) in &source_ids {
                    let snaps = super::store::list_snapshots(conn, Some(sid), 1000)?;
                    out.push((sid.clone(), label.clone(), kind.clone(), snaps.len()));
                }
                Ok(out)
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("join: {e}"))?
        .map_err(|e: anyhow::Error| anyhow::anyhow!("{e:#}"))?;

        let mut md = String::from("## Memory Sources (snapshot status)\n\n");
        if counts.is_empty() {
            md.push_str("No enabled memory sources configured.\n");
        } else {
            for (sid, label, kind, count) in &counts {
                md.push_str(&format!(
                    "- **{label}** ({kind}) — {count} snapshot(s) | source_id: `{sid}`\n"
                ));
            }
            md.push_str(
                "\nCall with `source_id` to see what changed since the last sync, \
                 or `checkpoint_id` for cross-source diffs.\n",
            );
        }

        Ok(ToolResult::success(md))
    }
}

fn format_diff_result(diff: &DiffResult) -> String {
    let mut md = format!(
        "## Memory Changes ({})\n\n**{} added, {} modified, {} removed** ({} unchanged)\n",
        diff.source_label,
        diff.summary.added,
        diff.summary.modified,
        diff.summary.removed,
        diff.summary.unchanged,
    );

    let added: Vec<_> = diff
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Added)
        .collect();
    let modified: Vec<_> = diff
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Modified)
        .collect();
    let removed: Vec<_> = diff
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Removed)
        .collect();

    if !added.is_empty() {
        md.push_str("\n### Added\n");
        for c in &added {
            let label = if c.title.is_empty() {
                &c.item_id
            } else {
                &c.title
            };
            md.push_str(&format!("- {label}\n"));
        }
    }

    if !modified.is_empty() {
        md.push_str("\n### Modified\n");
        for c in &modified {
            let label = if c.title.is_empty() {
                &c.item_id
            } else {
                &c.title
            };
            md.push_str(&format!("- {label}\n"));
            if let Some(diff_text) = &c.text_diff {
                md.push_str("  ```diff\n");
                for line in diff_text.lines() {
                    md.push_str(&format!("  {line}\n"));
                }
                md.push_str("  ```\n");
            }
        }
    }

    if !removed.is_empty() {
        md.push_str("\n### Removed\n");
        for c in &removed {
            let label = if c.title.is_empty() {
                &c.item_id
            } else {
                &c.title
            };
            md.push_str(&format!("- {label}\n"));
        }
    }

    if diff.changes.is_empty() {
        md.push_str("\nNo changes detected.\n");
    }

    md
}

fn format_cross_source_diff(diff: &CrossSourceDiff) -> String {
    let mut md = format!(
        "## Cross-Source Memory Changes\n\n\
         **Total: {} added, {} modified, {} removed** ({} unchanged)\n",
        diff.summary.added, diff.summary.modified, diff.summary.removed, diff.summary.unchanged,
    );

    if diff.per_source.is_empty() {
        md.push_str("\nNo changes across any source since the checkpoint.\n");
        return md;
    }

    for source_diff in &diff.per_source {
        md.push_str(&format!(
            "\n### {} ({})\n",
            source_diff.source_label, source_diff.source_kind
        ));
        md.push_str(&format!(
            "{} added, {} modified, {} removed\n",
            source_diff.summary.added, source_diff.summary.modified, source_diff.summary.removed,
        ));
        for c in &source_diff.changes {
            let label = if c.title.is_empty() {
                &c.item_id
            } else {
                &c.title
            };
            let prefix = match c.kind {
                ChangeKind::Added => "+",
                ChangeKind::Modified => "~",
                ChangeKind::Removed => "-",
            };
            md.push_str(&format!("  {prefix} {label}\n"));
        }
    }

    md
}
