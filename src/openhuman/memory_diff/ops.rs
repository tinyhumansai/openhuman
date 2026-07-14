//! Business logic for memory diff — thin host async wrappers over
//! `tinycortex::memory::diff::DiffEngine` (W7).
//!
//! The snapshot/diff/checkpoint/ledger engine is the crate's; the git ledger it
//! writes lives at the same `<workspace>/memory_diff/repo` path with the same
//! libgit2 layout, so existing ledgers keep working byte-for-byte. `DiffEngine`
//! is synchronous and generic over a chunk-source seam, so each op here builds
//! the host [`ChunkStoreItemSource`] (which reads the authoritative
//! `mem_tree_chunks`) and drives the engine inside `spawn_blocking`, preserving
//! the host's `async` + `Result<_, String>` signatures, the `DomainEvent`
//! publishes, and the tracing that RPC/tools/sync/subconscious callers expect.

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::MemorySourceEntry;

use tinycortex::memory::diff::{DiffEngine, SourceDescriptor};

use super::source::ChunkStoreItemSource;
use super::types::*;

/// A crate [`SourceDescriptor`] from a host source entry.
fn descriptor(source: &MemorySourceEntry) -> SourceDescriptor {
    SourceDescriptor::new(
        source.id.clone(),
        source.kind.as_str().to_string(),
        source.label.clone(),
    )
}

/// Take a snapshot of the current chunk-store state for a source.
///
/// Reads from `mem_tree_chunks` (already-ingested data) via the item-source
/// seam, groups by item, and commits one blob per item to the git ledger.
/// Returns the new [`Snapshot`] whose `id` is the commit SHA.
pub async fn take_snapshot(
    source: &MemorySourceEntry,
    config: &Config,
    trigger: SnapshotTrigger,
) -> Result<Snapshot, String> {
    let workspace_dir = config.workspace_dir.clone();
    let config_clone = config.clone();
    let source_owned = source.clone();
    let desc = descriptor(source);

    let snapshot = tokio::task::spawn_blocking(move || -> anyhow::Result<Snapshot> {
        let items = ChunkStoreItemSource::single(config_clone, &source_owned);
        let engine = DiffEngine::new(workspace_dir, items);
        engine.take_snapshot(&desc, trigger)
    })
    .await
    .map_err(|e| format!("snapshot join error: {e}"))?
    .map_err(|e: anyhow::Error| format!("take_snapshot: {e:#}"))?;

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
    let config_clone = config.clone();
    let to_id = to_snapshot_id.to_string();
    let from_id = from_snapshot_id.map(|s| s.to_string());

    tokio::task::spawn_blocking(move || -> anyhow::Result<DiffResult> {
        let engine = DiffEngine::new(workspace_dir, ChunkStoreItemSource::read_only(config_clone));
        engine.compute_diff(from_id.as_deref(), &to_id, include_text_diff)
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
    let config_clone = config.clone();
    let source_id = source.id.clone();

    tokio::task::spawn_blocking(move || -> anyhow::Result<DiffResult> {
        let engine = DiffEngine::new(workspace_dir, ChunkStoreItemSource::read_only(config_clone));
        engine.diff_since_last(&source_id, include_text_diff)
    })
    .await
    .map_err(|e| format!("diff_since_last join: {e}"))?
    .map_err(|e: anyhow::Error| format!("diff_since_last: {e:#}"))
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
    let config_clone = config.clone();
    let source_id = source.id.clone();

    let diff = tokio::task::spawn_blocking(move || -> anyhow::Result<DiffResult> {
        let engine = DiffEngine::new(workspace_dir, ChunkStoreItemSource::read_only(config_clone));
        engine.diff_since_read(&source_id, include_text_diff, commit)
    })
    .await
    .map_err(|e| format!("diff_since_read join: {e}"))?
    .map_err(|e: anyhow::Error| format!("diff_since_read: {e:#}"))?;

    if commit {
        tracing::debug!(
            source_id = %source.id,
            snapshot_id = %diff.to_snapshot_id,
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
    let config_clone = config.clone();
    let ids_for_blocking = target_ids.clone();

    let (marked, snapshot_ids) =
        tokio::task::spawn_blocking(move || -> anyhow::Result<(u64, Vec<String>)> {
            let engine =
                DiffEngine::new(workspace_dir, ChunkStoreItemSource::read_only(config_clone));
            // Gather the head snapshot ids that will be marked, for the event
            // payload (the crate `mark_read` returns only a count).
            let mut snapshot_ids = Vec::new();
            for sid in &ids_for_blocking {
                if let Some(head) = engine.list_snapshots(Some(sid), 1)?.into_iter().next() {
                    snapshot_ids.push(head.id);
                }
            }
            let marked = engine.mark_read(&ids_for_blocking)?;
            Ok((marked, snapshot_ids))
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
/// enabled source. Sources lacking a snapshot are baselined first.
pub async fn create_checkpoint(label: &str, config: &Config) -> Result<Checkpoint, String> {
    let sources = crate::openhuman::memory_sources::registry::list_sources()
        .await
        .map_err(|e| format!("list sources: {e}"))?;
    let enabled: Vec<MemorySourceEntry> = sources.into_iter().filter(|s| s.enabled).collect();

    let workspace_dir = config.workspace_dir.clone();
    let config_clone = config.clone();
    let label_owned = label.to_string();

    let checkpoint = tokio::task::spawn_blocking(move || -> anyhow::Result<Checkpoint> {
        let descriptors: Vec<SourceDescriptor> = enabled.iter().map(descriptor).collect();
        let items = ChunkStoreItemSource::for_sources(config_clone, &enabled);
        let engine = DiffEngine::new(workspace_dir, items);
        engine.create_checkpoint(&label_owned, &descriptors)
    })
    .await
    .map_err(|e| format!("checkpoint persist join: {e}"))?
    .map_err(|e: anyhow::Error| format!("create_checkpoint: {e:#}"))?;

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
    let config_clone = config.clone();
    let ckpt_id = checkpoint_id.to_string();

    tokio::task::spawn_blocking(move || -> anyhow::Result<CrossSourceDiff> {
        let engine = DiffEngine::new(workspace_dir, ChunkStoreItemSource::read_only(config_clone));
        engine.diff_since_checkpoint(&ckpt_id, include_text_diff)
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
    let config_clone = config.clone();

    tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
        let engine = DiffEngine::new(workspace_dir, ChunkStoreItemSource::read_only(config_clone));
        engine.cleanup(older_than_days)
    })
    .await
    .map_err(|e| format!("cleanup join: {e}"))?
    .map_err(|e: anyhow::Error| format!("cleanup: {e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinycortex::memory::diff::{Ledger, SnapshotMeta};

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
            kind: crate::openhuman::memory_sources::types::SourceKind::Folder,
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

    /// Seed a snapshot directly through the (crate) ledger, bypassing the chunk
    /// store — exercises the host async wrappers over real ledger state.
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
