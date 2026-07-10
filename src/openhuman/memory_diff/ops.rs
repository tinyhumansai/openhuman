//! Business logic for memory diff: snapshot capture, diff computation,
//! checkpoints, and cleanup — all backed by the git ledger (`git_store`).
//!
//! `mem_tree_chunks` remains authoritative. Each `take_snapshot` materialises a
//! source's current items as git blobs and records them as a commit; diffs are
//! git tree diffs, checkpoints are tags, read markers are refs.

use std::collections::HashMap;

use anyhow::{anyhow, bail};

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_store::chunks::store as chunk_store;

use super::git_store::{Ledger, SnapshotMeta};
use super::types::*;

/// Take a snapshot of the current chunk-store state for a source.
///
/// Reads from `mem_tree_chunks` (already-ingested data), groups by item, and
/// commits one blob per item to the git ledger. Returns the new [`Snapshot`]
/// whose `id` is the commit SHA.
pub async fn take_snapshot(
    source: &MemorySourceEntry,
    config: &Config,
    trigger: SnapshotTrigger,
) -> Result<Snapshot, String> {
    let prefix = source_id_prefix(source);
    let config_clone = config.clone();

    // Group chunk content per item, in chunk order, into (item_id, content).
    let items = tokio::task::spawn_blocking(move || {
        chunk_store::with_connection(&config_clone, |conn| {
            let mut stmt = conn.prepare(
                "SELECT source_id, content \
                 FROM mem_tree_chunks \
                 WHERE source_id LIKE ?1 \
                 ORDER BY source_id, seq_in_source",
            )?;

            let mut groups: HashMap<String, Vec<String>> = HashMap::new();
            let rows = stmt.query_map([&prefix], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (composite_source_id, content) = row?;
                let item_id = extract_item_id(&composite_source_id);
                groups.entry(item_id).or_default().push(content);
            }

            let mut items: Vec<(String, String)> = groups
                .into_iter()
                .map(|(item_id, parts)| (item_id, parts.join("")))
                .collect();
            items.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(items)
        })
    })
    .await
    .map_err(|e| format!("snapshot join error: {e}"))?
    .map_err(|e: anyhow::Error| format!("snapshot query error: {e:#}"))?;

    let meta = SnapshotMeta {
        source_id: source.id.clone(),
        source_kind: source.kind.as_str().to_string(),
        label: source.label.clone(),
        trigger,
    };
    let workspace_dir = config.workspace_dir.clone();
    let now_ms = chrono::Utc::now().timestamp_millis();

    let snapshot = tokio::task::spawn_blocking(move || -> anyhow::Result<Snapshot> {
        let ledger = Ledger::open(&workspace_dir)?;
        ledger.commit_snapshot(&meta, &items, now_ms)
    })
    .await
    .map_err(|e| format!("snapshot persist join: {e}"))?
    .map_err(|e: anyhow::Error| format!("snapshot persist: {e:#}"))?;

    tracing::debug!(
        snapshot_id = %snapshot.id,
        source_id = %source.id,
        items = snapshot.item_count,
        trigger = %snapshot.trigger.as_str(),
        "[memory_diff] snapshot taken"
    );

    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::MemoryDiffSnapshotTaken {
            snapshot_id: snapshot.id.clone(),
            source_id: source.id.clone(),
            source_kind: source.kind.as_str().to_string(),
            item_count: snapshot.item_count as usize,
            trigger: snapshot.trigger.as_str().to_string(),
        },
    );

    Ok(snapshot)
}

/// Auto-snapshot hook called from `sync_source()` after a successful sync.
pub async fn auto_snapshot_after_sync(
    source: &MemorySourceEntry,
    config: &Config,
) -> Result<Snapshot, String> {
    take_snapshot(source, config, SnapshotTrigger::Auto).await
}

/// Compute the diff between two snapshots of the same source.
pub async fn compute_diff(
    config: &Config,
    from_snapshot_id: Option<&str>,
    to_snapshot_id: &str,
    include_text_diff: bool,
) -> Result<DiffResult, String> {
    let workspace_dir = config.workspace_dir.clone();
    let to_id = to_snapshot_id.to_string();
    let from_id = from_snapshot_id.map(|s| s.to_string());

    tokio::task::spawn_blocking(move || -> anyhow::Result<DiffResult> {
        let ledger = Ledger::open(&workspace_dir)?;
        let to_snap = ledger
            .get_snapshot(&to_id)?
            .ok_or_else(|| anyhow!("snapshot not found: {to_id}"))?;

        let from_snap = match &from_id {
            Some(fid) => {
                let s = ledger
                    .get_snapshot(fid)?
                    .ok_or_else(|| anyhow!("snapshot not found: {fid}"))?;
                if s.source_id != to_snap.source_id {
                    bail!(
                        "cross-source diff not allowed: from={} to={}",
                        s.source_id,
                        to_snap.source_id
                    );
                }
                Some(s)
            }
            None => None,
        };

        let (changes, summary) = ledger.compute_changes(
            from_id.as_deref(),
            &to_id,
            &to_snap.source_id,
            to_snap.item_count,
            include_text_diff,
        )?;

        Ok(DiffResult {
            source_id: to_snap.source_id.clone(),
            source_kind: to_snap.source_kind.clone(),
            source_label: to_snap.label.clone(),
            from_snapshot_id: from_snap.map(|s| s.id),
            to_snapshot_id: to_snap.id.clone(),
            summary,
            changes,
        })
    })
    .await
    .map_err(|e| format!("diff join: {e}"))?
    .map_err(|e: anyhow::Error| format!("compute_diff: {e:#}"))
}

/// Diff current state (latest snapshot) vs previous snapshot for a source.
pub async fn diff_since_last(
    source: &MemorySourceEntry,
    config: &Config,
    include_text_diff: bool,
) -> Result<DiffResult, String> {
    let workspace_dir = config.workspace_dir.clone();
    let source_id = source.id.clone();

    let snapshots = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Snapshot>> {
        let ledger = Ledger::open(&workspace_dir)?;
        ledger.latest_snapshots_for_source(&source_id, 2)
    })
    .await
    .map_err(|e| format!("diff_since_last join: {e}"))?
    .map_err(|e: anyhow::Error| format!("diff_since_last: {e:#}"))?;

    match snapshots.len() {
        0 => Err("no snapshots found for this source".to_string()),
        1 => compute_diff(config, None, &snapshots[0].id, include_text_diff).await,
        _ => {
            compute_diff(
                config,
                Some(&snapshots[1].id),
                &snapshots[0].id,
                include_text_diff,
            )
            .await
        }
    }
}

/// Diff a source's latest snapshot against its read marker — i.e. everything
/// that changed since the agent last *read* this source's diff.
///
/// When `commit` is true, the read marker (a git ref) is advanced to the head
/// snapshot after the diff is computed, so a subsequent call returns only newer
/// changes. This is the turn-to-turn primitive: read the world delta, then
/// acknowledge it as consumed.
pub async fn diff_since_read(
    source: &MemorySourceEntry,
    config: &Config,
    include_text_diff: bool,
    commit: bool,
) -> Result<DiffResult, String> {
    let workspace_dir = config.workspace_dir.clone();
    let source_id = source.id.clone();

    // Resolve head (latest snapshot) and the marker's base snapshot. If the
    // marker points at a commit that no longer resolves, treat it as unread.
    let (head, base_id) = tokio::task::spawn_blocking(
        move || -> anyhow::Result<(Option<Snapshot>, Option<String>)> {
            let ledger = Ledger::open(&workspace_dir)?;
            let head = ledger
                .latest_snapshots_for_source(&source_id, 1)?
                .into_iter()
                .next();
            let marker = ledger.get_read_marker(&source_id)?;
            let base_id = match marker {
                Some(snap_id) if ledger.get_snapshot(&snap_id)?.is_some() => Some(snap_id),
                _ => None,
            };
            Ok((head, base_id))
        },
    )
    .await
    .map_err(|e| format!("diff_since_read join: {e}"))?
    .map_err(|e: anyhow::Error| format!("diff_since_read: {e:#}"))?;

    let head = head.ok_or_else(|| "no snapshots found for this source".to_string())?;

    let diff = compute_diff(config, base_id.as_deref(), &head.id, include_text_diff).await?;

    if commit {
        let workspace_dir = config.workspace_dir.clone();
        let source_id = source.id.clone();
        let head_id = head.id.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let ledger = Ledger::open(&workspace_dir)?;
            ledger.set_read_marker(&source_id, &head_id)
        })
        .await
        .map_err(|e| format!("diff_since_read commit join: {e}"))?
        .map_err(|e: anyhow::Error| format!("diff_since_read commit: {e:#}"))?;

        tracing::debug!(
            source_id = %source.id,
            snapshot_id = %head.id,
            added = diff.summary.added,
            modified = diff.summary.modified,
            removed = diff.summary.removed,
            "[memory_diff] read marker committed"
        );
    }

    Ok(diff)
}

/// Commit a read marker for one or more sources, advancing each to its
/// current head snapshot. When `source_ids` is `None`, marks all enabled
/// sources that have at least one snapshot. Returns the number of markers set.
pub async fn mark_read(config: &Config, source_ids: Option<Vec<String>>) -> Result<u64, String> {
    let target_ids: Vec<String> = match source_ids {
        Some(ids) => ids,
        None => crate::openhuman::memory_sources::registry::list_sources()
            .await
            .map_err(|e| format!("list sources: {e}"))?
            .into_iter()
            .filter(|s| s.enabled)
            .map(|s| s.id)
            .collect(),
    };

    let workspace_dir = config.workspace_dir.clone();
    let ids_for_blocking = target_ids.clone();
    let (marked, snapshot_ids) =
        tokio::task::spawn_blocking(move || -> anyhow::Result<(u64, Vec<String>)> {
            let ledger = Ledger::open(&workspace_dir)?;
            let mut count = 0u64;
            let mut snapshot_ids = Vec::new();
            for sid in &ids_for_blocking {
                if let Some(head) = ledger
                    .latest_snapshots_for_source(sid, 1)?
                    .into_iter()
                    .next()
                {
                    ledger.set_read_marker(sid, &head.id)?;
                    snapshot_ids.push(head.id);
                    count += 1;
                }
            }
            Ok((count, snapshot_ids))
        })
        .await
        .map_err(|e| format!("mark_read join: {e}"))?
        .map_err(|e: anyhow::Error| format!("mark_read: {e:#}"))?;

    tracing::debug!(
        sources = marked,
        "[memory_diff] mark_read committed read markers"
    );

    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::MemoryDiffMarkedRead {
            source_ids: target_ids,
            snapshot_ids,
        },
    );

    Ok(marked)
}

/// Create a checkpoint (git tag at HEAD) grouping the latest snapshot per
/// enabled source.
pub async fn create_checkpoint(label: &str, config: &Config) -> Result<Checkpoint, String> {
    let sources = crate::openhuman::memory_sources::registry::list_sources()
        .await
        .map_err(|e| format!("list sources: {e}"))?;
    let enabled: Vec<_> = sources.into_iter().filter(|s| s.enabled).collect();

    // Take a snapshot for any source that doesn't have one yet, so the
    // checkpoint has a baseline for every source.
    let workspace_dir = config.workspace_dir.clone();
    let enabled_ids: Vec<String> = enabled.iter().map(|s| s.id.clone()).collect();
    let ids_clone = enabled_ids.clone();
    let lacking = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<String>> {
        let ledger = Ledger::open(&workspace_dir)?;
        let mut lacking = Vec::new();
        for sid in &ids_clone {
            if ledger.snapshot_count_for_source(sid)? == 0 {
                lacking.push(sid.clone());
            }
        }
        Ok(lacking)
    })
    .await
    .map_err(|e| format!("checkpoint check join: {e}"))?
    .map_err(|e: anyhow::Error| format!("checkpoint check: {e:#}"))?;

    for source in enabled.iter().filter(|s| lacking.contains(&s.id)) {
        take_snapshot(source, config, SnapshotTrigger::Manual).await?;
    }

    // Gather the latest snapshot id per source, then tag HEAD.
    let workspace_dir = config.workspace_dir.clone();
    let checkpoint_id = format!("ckpt_{}", uuid::Uuid::new_v4());
    let created_at_ms = chrono::Utc::now().timestamp_millis();
    let label_owned = label.to_string();
    let ckpt_id_clone = checkpoint_id.clone();

    let checkpoint = tokio::task::spawn_blocking(move || -> anyhow::Result<Checkpoint> {
        let ledger = Ledger::open(&workspace_dir)?;
        let mut snapshot_ids = Vec::new();
        for sid in &enabled_ids {
            if let Some(snap) = ledger
                .latest_snapshots_for_source(sid, 1)?
                .into_iter()
                .next()
            {
                snapshot_ids.push(snap.id);
            }
        }
        ledger.create_checkpoint(&ckpt_id_clone, &label_owned, &snapshot_ids, created_at_ms)?;
        Ok(Checkpoint {
            id: ckpt_id_clone,
            label: label_owned,
            created_at_ms,
            snapshot_ids,
        })
    })
    .await
    .map_err(|e| format!("checkpoint persist join: {e}"))?
    .map_err(|e: anyhow::Error| format!("checkpoint persist: {e:#}"))?;

    tracing::debug!(
        checkpoint_id = %checkpoint.id,
        snapshots = checkpoint.snapshot_ids.len(),
        "[memory_diff] checkpoint created"
    );

    Ok(checkpoint)
}

/// Compute a cross-source diff: everything that changed since a checkpoint.
pub async fn diff_since_checkpoint(
    checkpoint_id: &str,
    config: &Config,
    include_text_diff: bool,
) -> Result<CrossSourceDiff, String> {
    let workspace_dir = config.workspace_dir.clone();
    let ckpt_id = checkpoint_id.to_string();
    let computed_at_ms = chrono::Utc::now().timestamp_millis();

    tokio::task::spawn_blocking(move || -> anyhow::Result<CrossSourceDiff> {
        let ledger = Ledger::open(&workspace_dir)?;
        let checkpoint = ledger
            .get_checkpoint(&ckpt_id)?
            .ok_or_else(|| anyhow!("checkpoint not found: {ckpt_id}"))?;

        let mut per_source = Vec::new();
        let mut agg = DiffSummary::default();

        for snap_id in &checkpoint.snapshot_ids {
            let Some(base) = ledger.get_snapshot(snap_id)? else {
                continue;
            };
            let Some(head) = ledger
                .latest_snapshots_for_source(&base.source_id, 1)?
                .into_iter()
                .next()
            else {
                continue;
            };
            if head.id == base.id {
                continue; // unchanged since the checkpoint
            }

            let (changes, summary) = ledger.compute_changes(
                Some(&base.id),
                &head.id,
                &head.source_id,
                head.item_count,
                include_text_diff,
            )?;
            agg.added += summary.added;
            agg.removed += summary.removed;
            agg.modified += summary.modified;
            agg.unchanged += summary.unchanged;
            per_source.push(DiffResult {
                source_id: head.source_id.clone(),
                source_kind: head.source_kind.clone(),
                source_label: head.label.clone(),
                from_snapshot_id: Some(base.id.clone()),
                to_snapshot_id: head.id.clone(),
                summary,
                changes,
            });
        }

        Ok(CrossSourceDiff {
            checkpoint_id: Some(checkpoint.id),
            computed_at_ms,
            summary: agg,
            per_source,
        })
    })
    .await
    .map_err(|e| format!("diff_since_checkpoint join: {e}"))?
    .map_err(|e: anyhow::Error| format!("diff_since_checkpoint: {e:#}"))
}

/// Delete checkpoint tags older than `older_than_days`.
///
/// Snapshot commits are retained — git history *is* the ledger, and git's
/// delta compression keeps it compact — so cleanup only prunes named baselines.
/// Returns the number of checkpoints deleted.
pub async fn cleanup(config: &Config, older_than_days: u32) -> Result<u64, String> {
    let workspace_dir = config.workspace_dir.clone();
    let cutoff =
        chrono::Utc::now().timestamp_millis() - (older_than_days as i64 * 24 * 60 * 60 * 1000);

    tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
        let ledger = Ledger::open(&workspace_dir)?;
        ledger.cleanup_checkpoints(cutoff)
    })
    .await
    .map_err(|e| format!("cleanup join: {e}"))?
    .map_err(|e: anyhow::Error| format!("cleanup: {e:#}"))
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Build the `source_id LIKE` prefix that matches chunks belonging to a source.
/// Mirrors `memory_sources::status::source_id_prefix`.
fn source_id_prefix(source: &MemorySourceEntry) -> String {
    match source.kind {
        SourceKind::Composio => source
            .toolkit
            .as_deref()
            .map(|t| format!("{t}:%"))
            .unwrap_or_else(|| "__no_toolkit__:%".to_string()),
        _ => format!("mem_src:{}:%", source.id),
    }
}

/// Extract the item-level id from a composite chunk source_id.
///
/// For reader-backed: `mem_src:src_abc:readme.md` → `readme.md`
/// For Composio: `gmail:user@example.com:msg_xxx` → `user@example.com:msg_xxx`
fn extract_item_id(composite: &str) -> String {
    if let Some(rest) = composite.strip_prefix("mem_src:") {
        // Skip the source id segment
        if let Some(pos) = rest.find(':') {
            return rest[pos + 1..].to_string();
        }
    }
    // Composio or other: strip first segment
    if let Some(pos) = composite.find(':') {
        return composite[pos + 1..].to_string();
    }
    composite.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_diff::git_store::Ledger;

    #[test]
    fn extract_item_id_reader_backed() {
        assert_eq!(extract_item_id("mem_src:src_abc:readme.md"), "readme.md");
        assert_eq!(
            extract_item_id("mem_src:src_abc:path/to/file.md"),
            "path/to/file.md"
        );
    }

    #[test]
    fn extract_item_id_composio() {
        assert_eq!(
            extract_item_id("gmail:user@example.com:msg_xxx"),
            "user@example.com:msg_xxx"
        );
    }

    #[test]
    fn extract_item_id_no_prefix() {
        assert_eq!(extract_item_id("standalone"), "standalone");
    }

    #[test]
    fn source_id_prefix_folder() {
        assert_eq!(
            source_id_prefix(&folder_source("src_abc")),
            "mem_src:src_abc:%"
        );
    }

    #[test]
    fn source_id_prefix_composio() {
        let mut entry = folder_source("src_cmp");
        entry.kind = SourceKind::Composio;
        entry.toolkit = Some("gmail".into());
        assert_eq!(source_id_prefix(&entry), "gmail:%");
    }

    // ── Integration-style ops tests over a temp git ledger ────────────────

    fn test_config() -> Config {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        // Leak the tempdir so the path stays valid for the test's lifetime.
        std::mem::forget(dir);
        config
    }

    fn folder_source(id: &str) -> MemorySourceEntry {
        MemorySourceEntry {
            id: id.into(),
            kind: SourceKind::Folder,
            label: "Docs".into(),
            enabled: true,
            toolkit: None,
            connection_id: None,
            path: Some("/tmp".into()),
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            query: None,
            since_days: None,
            max_items: None,
            max_commits: None,
            max_issues: None,
            max_prs: None,
            selector: None,
            max_tokens_per_sync: None,
            max_cost_per_sync_usd: None,
            sync_depth_days: None,
        }
    }

    /// Seed a snapshot directly through the ledger (bypassing the chunk store).
    fn seed(
        config: &Config,
        source_id: &str,
        taken_at_ms: i64,
        items: &[(&str, &str)],
    ) -> Snapshot {
        let ledger = Ledger::open(&config.workspace_dir).unwrap();
        let items: Vec<(String, String)> = items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ledger
            .commit_snapshot(
                &SnapshotMeta {
                    source_id: source_id.to_string(),
                    source_kind: "folder".to_string(),
                    label: "Docs".to_string(),
                    trigger: SnapshotTrigger::Auto,
                },
                &items,
                taken_at_ms,
            )
            .unwrap()
    }

    #[tokio::test]
    async fn compute_diff_detects_added_modified_removed() {
        let config = test_config();
        let from = seed(
            &config,
            "src_a",
            1000,
            &[("a", "alpha"), ("b", "beta"), ("c", "gamma")],
        );
        let to = seed(
            &config,
            "src_a",
            2000,
            &[("a", "alpha"), ("b", "beta v2"), ("d", "delta")],
        );

        let diff = compute_diff(&config, Some(&from.id), &to.id, false)
            .await
            .unwrap();

        assert_eq!(diff.summary.added, 1, "d added");
        assert_eq!(diff.summary.modified, 1, "b modified");
        assert_eq!(diff.summary.removed, 1, "c removed");
        assert_eq!(diff.summary.unchanged, 1, "a unchanged");

        let kind_of = |id: &str| {
            diff.changes
                .iter()
                .find(|c| c.item_id == id)
                .map(|c| c.kind.clone())
        };
        assert_eq!(kind_of("d"), Some(ChangeKind::Added));
        assert_eq!(kind_of("b"), Some(ChangeKind::Modified));
        assert_eq!(kind_of("c"), Some(ChangeKind::Removed));
        assert_eq!(kind_of("a"), None, "unchanged items are not in changes");
    }

    #[tokio::test]
    async fn compute_diff_against_none_marks_all_added() {
        let config = test_config();
        let to = seed(&config, "src_a", 1000, &[("a", "x")]);
        let diff = compute_diff(&config, None, &to.id, false).await.unwrap();
        assert_eq!(diff.summary.added, 1);
        assert_eq!(diff.from_snapshot_id, None);
    }

    #[tokio::test]
    async fn compute_diff_rejects_cross_source() {
        let config = test_config();
        let from = seed(&config, "src_a", 1000, &[("a", "x")]);
        let to = seed(&config, "src_b", 2000, &[("b", "y")]);
        let err = compute_diff(&config, Some(&from.id), &to.id, false)
            .await
            .unwrap_err();
        assert!(err.contains("cross-source"), "got: {err}");
    }

    #[tokio::test]
    async fn compute_diff_text_diff_only_when_requested() {
        let config = test_config();
        let from = seed(&config, "src_a", 1000, &[("a", "line one\nline two\n")]);
        let to = seed(
            &config,
            "src_a",
            2000,
            &[("a", "line one\nline TWO changed\n")],
        );

        let without = compute_diff(&config, Some(&from.id), &to.id, false)
            .await
            .unwrap();
        assert!(without.changes[0].text_diff.is_none());

        let with = compute_diff(&config, Some(&from.id), &to.id, true)
            .await
            .unwrap();
        let td = with.changes[0]
            .text_diff
            .as_ref()
            .expect("text diff present");
        assert!(td.contains("line TWO changed"), "got: {td}");
    }

    #[tokio::test]
    async fn diff_since_last_handles_zero_one_two_snapshots() {
        let config = test_config();
        let source = folder_source("src_a");

        // 0 snapshots → error
        assert!(diff_since_last(&source, &config, false).await.is_err());

        // 1 snapshot → everything added (diff vs None)
        seed(&config, "src_a", 1000, &[("a", "x")]);
        let one = diff_since_last(&source, &config, false).await.unwrap();
        assert_eq!(one.summary.added, 1);

        // 2 snapshots → diff latest vs previous
        seed(&config, "src_a", 2000, &[("a", "x"), ("b", "y")]);
        let two = diff_since_last(&source, &config, false).await.unwrap();
        assert_eq!(two.summary.added, 1, "b is new in s2");
        assert_eq!(two.summary.unchanged, 1, "a unchanged");
    }

    #[tokio::test]
    async fn diff_since_read_commits_marker_and_returns_only_new_changes() {
        let config = test_config();
        let source = folder_source("src_a");

        seed(&config, "src_a", 1000, &[("a", "x")]);

        // First read: no marker → full diff (a added), and commit advances marker.
        let first = diff_since_read(&source, &config, false, true)
            .await
            .unwrap();
        assert_eq!(first.summary.added, 1);

        // Second read with no new snapshot: marker == head → nothing changed.
        let second = diff_since_read(&source, &config, false, true)
            .await
            .unwrap();
        assert_eq!(second.summary.added, 0);
        assert_eq!(second.summary.modified, 0);
        assert_eq!(second.summary.removed, 0);
        assert!(second.changes.is_empty());

        // New snapshot then read: only the delta since the marker shows.
        seed(&config, "src_a", 2000, &[("a", "x"), ("b", "y")]);
        let third = diff_since_read(&source, &config, false, true)
            .await
            .unwrap();
        assert_eq!(third.summary.added, 1, "only b is new since last read");
        assert_eq!(third.summary.unchanged, 1);
    }

    #[tokio::test]
    async fn diff_since_read_without_commit_does_not_advance_marker() {
        let config = test_config();
        let source = folder_source("src_a");
        seed(&config, "src_a", 1000, &[("a", "x")]);

        // Preview (commit=false) twice → both show the full diff.
        let a = diff_since_read(&source, &config, false, false)
            .await
            .unwrap();
        let b = diff_since_read(&source, &config, false, false)
            .await
            .unwrap();
        assert_eq!(a.summary.added, 1);
        assert_eq!(b.summary.added, 1, "marker was not advanced");
    }

    #[tokio::test]
    async fn mark_read_advances_marker_for_explicit_sources() {
        let config = test_config();
        let source = folder_source("src_a");
        seed(&config, "src_a", 1000, &[("a", "x")]);

        let marked = mark_read(&config, Some(vec!["src_a".to_string()]))
            .await
            .unwrap();
        assert_eq!(marked, 1);

        // After marking, a read shows no changes (marker already at head).
        let diff = diff_since_read(&source, &config, false, false)
            .await
            .unwrap();
        assert_eq!(diff.summary.added, 0);
        assert!(diff.changes.is_empty());
    }

    #[tokio::test]
    async fn diff_since_checkpoint_aggregates_across_sources() {
        let config = test_config();
        // Baseline snapshots for two sources, grouped into a checkpoint.
        let a1 = seed(&config, "src_a", 1000, &[("a", "x")]);
        let b1 = seed(&config, "src_b", 1000, &[("b", "y")]);
        {
            let ledger = Ledger::open(&config.workspace_dir).unwrap();
            ledger
                .create_checkpoint("ckpt_1", "base", &[a1.id.clone(), b1.id.clone()], 1500)
                .unwrap();
        }

        // src_a gets a new head with a modification; src_b unchanged.
        seed(&config, "src_a", 2000, &[("a", "x v2")]);

        let cross = diff_since_checkpoint("ckpt_1", &config, false)
            .await
            .unwrap();
        assert_eq!(cross.summary.modified, 1, "src_a 'a' modified");
        assert_eq!(
            cross.per_source.len(),
            1,
            "only src_a changed; unchanged src_b is skipped"
        );
        assert_eq!(cross.per_source[0].source_id, "src_a");
    }
}
