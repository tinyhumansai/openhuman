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
        "Check what changed in memory sources since you last looked, the last sync, or a named \
         checkpoint. Returns a structured summary of added, removed, and modified items. By \
         default, reading a single source's diff commits a read marker so the next call only \
         surfaces newer changes (set commit=false to preview without acknowledging)."
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
                },
                "since_read": {
                    "type": "boolean",
                    "description": "When diffing a single source, show changes since you last read \
                                    this source's diff (vs. since the previous sync). Default true.",
                    "default": true
                },
                "commit": {
                    "type": "boolean",
                    "description": "When using since_read, advance the read marker so the next call \
                                    only surfaces newer changes. Default true; set false to preview.",
                    "default": true
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Read-only with respect to the user's data: the only write this tool
        // performs is advancing the read marker in the module's own diff.db
        // (internal bookkeeping under workspace state, never `action_dir`).
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let source_id = args.get("source_id").and_then(|v| v.as_str());
        let checkpoint_id = args.get("checkpoint_id").and_then(|v| v.as_str());
        let include_text_diff = args
            .get("include_text_diff")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let since_read = args
            .get("since_read")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let commit = args.get("commit").and_then(|v| v.as_bool()).unwrap_or(true);

        debug!(
            "[memory_diff][tool] execute source_id={:?} checkpoint_id={:?} include_text_diff={} \
             since_read={} commit={}",
            source_id, checkpoint_id, include_text_diff, since_read, commit
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

            let diff = if since_read {
                ops::diff_since_read(&source, &config, include_text_diff, commit)
                    .await
                    .map_err(|e| anyhow::anyhow!(e))?
            } else {
                ops::diff_since_last(&source, &config, include_text_diff)
                    .await
                    .map_err(|e| anyhow::anyhow!(e))?
            };
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

        let counts: Vec<(String, String, String, usize)> =
            tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let ledger = super::git_store::Ledger::open(&workspace_dir)?;
                let mut out = Vec::new();
                for (sid, label, kind) in &source_ids {
                    let count = ledger.snapshot_count_for_source(sid)?;
                    out.push((sid.clone(), label.clone(), kind.clone(), count));
                }
                Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn change(item_id: &str, title: &str, kind: ChangeKind, text_diff: Option<&str>) -> ItemChange {
        ItemChange {
            item_id: item_id.to_string(),
            title: title.to_string(),
            kind,
            old_content_hash: None,
            new_content_hash: None,
            text_diff: text_diff.map(str::to_string),
        }
    }

    #[test]
    fn format_diff_result_groups_changes_and_renders_text_diff() {
        let diff = DiffResult {
            source_id: "src_a".into(),
            source_kind: "folder".into(),
            source_label: "Docs".into(),
            from_snapshot_id: Some("s1".into()),
            to_snapshot_id: "s2".into(),
            summary: DiffSummary {
                added: 1,
                removed: 1,
                modified: 1,
                unchanged: 2,
            },
            changes: vec![
                change("new.md", "New Doc", ChangeKind::Added, None),
                change(
                    "edit.md",
                    "Edited Doc",
                    ChangeKind::Modified,
                    Some("@@ -1 +1 @@\n-old\n+new"),
                ),
                // Empty title falls back to the item id.
                change("gone.md", "", ChangeKind::Removed, None),
            ],
        };

        let md = format_diff_result(&diff);
        assert!(md.contains("1 added, 1 modified, 1 removed"));
        assert!(md.contains("### Added\n- New Doc"));
        assert!(md.contains("### Modified\n- Edited Doc"));
        assert!(md.contains("```diff"), "text diff should be fenced: {md}");
        assert!(md.contains("+new"));
        assert!(
            md.contains("### Removed\n- gone.md"),
            "title falls back to id"
        );
    }

    #[test]
    fn format_diff_result_reports_no_changes() {
        let diff = DiffResult {
            source_id: "src_a".into(),
            source_kind: "folder".into(),
            source_label: "Docs".into(),
            from_snapshot_id: Some("s1".into()),
            to_snapshot_id: "s2".into(),
            summary: DiffSummary::default(),
            changes: vec![],
        };
        assert!(format_diff_result(&diff).contains("No changes detected."));
    }

    #[test]
    fn format_cross_source_diff_breaks_down_per_source() {
        let cross = CrossSourceDiff {
            checkpoint_id: Some("ckpt_1".into()),
            computed_at_ms: 0,
            summary: DiffSummary {
                added: 1,
                modified: 0,
                removed: 0,
                unchanged: 0,
            },
            per_source: vec![DiffResult {
                source_id: "src_a".into(),
                source_kind: "folder".into(),
                source_label: "Docs".into(),
                from_snapshot_id: Some("s1".into()),
                to_snapshot_id: "s2".into(),
                summary: DiffSummary {
                    added: 1,
                    ..Default::default()
                },
                changes: vec![change("new.md", "New Doc", ChangeKind::Added, None)],
            }],
        };
        let md = format_cross_source_diff(&cross);
        assert!(md.contains("Total: 1 added"));
        assert!(md.contains("### Docs (folder)"));
        assert!(md.contains("+ New Doc"));
    }

    #[test]
    fn format_cross_source_diff_empty_is_explicit() {
        let cross = CrossSourceDiff {
            checkpoint_id: Some("ckpt_1".into()),
            computed_at_ms: 0,
            summary: DiffSummary::default(),
            per_source: vec![],
        };
        assert!(format_cross_source_diff(&cross).contains("No changes across any source"));
    }
}
