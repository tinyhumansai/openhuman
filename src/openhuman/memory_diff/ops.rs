//! Business logic for memory diff: snapshot capture, diff computation,
//! checkpoints, and cleanup.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_store::chunks::store as chunk_store;

use super::store;
use super::types::*;

const DEFAULT_RETENTION_DAYS: u32 = 30;
const MAX_SNAPSHOTS_PER_SOURCE: u32 = 100;
const MAX_TEXT_DIFF_CHARS: usize = 2000;

/// Take a snapshot of the current chunk-store state for a source.
///
/// Reads from `mem_tree_chunks` (already-ingested data), groups by item,
/// hashes content, and persists to the diff database.
pub async fn take_snapshot(
    source: &MemorySourceEntry,
    config: &Config,
    trigger: SnapshotTrigger,
) -> Result<Snapshot, String> {
    let source_clone = source.clone();
    let config_clone = config.clone();
    let prefix = source_id_prefix(&source_clone);

    let items = tokio::task::spawn_blocking(move || {
        chunk_store::with_connection(&config_clone, |conn| {
            let mut stmt = conn.prepare(
                "SELECT source_id, content, timestamp_ms \
                 FROM mem_tree_chunks \
                 WHERE source_id LIKE ?1 \
                 ORDER BY source_id, seq_in_source",
            )?;

            let mut groups: HashMap<String, ItemAccumulator> = HashMap::new();
            let rows = stmt.query_map([&prefix], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;

            for row in rows {
                let (composite_source_id, content, ts) = row?;
                let item_id = extract_item_id(&composite_source_id);
                let acc = groups.entry(item_id).or_default();
                acc.content_parts.push(content);
                acc.max_timestamp_ms = acc.max_timestamp_ms.max(Some(ts));
                acc.chunk_count += 1;
            }

            let mut snapshot_items: Vec<SnapshotItem> = groups
                .into_iter()
                .map(|(item_id, acc)| {
                    let concat = acc.content_parts.join("");
                    let hash = sha256_hex(concat.as_bytes());
                    SnapshotItem {
                        item_id,
                        title: String::new(),
                        content_hash: hash,
                        timestamp_ms: acc.max_timestamp_ms,
                        chunk_count: acc.chunk_count,
                    }
                })
                .collect();
            snapshot_items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
            Ok(snapshot_items)
        })
    })
    .await
    .map_err(|e| format!("snapshot join error: {e}"))?
    .map_err(|e: anyhow::Error| format!("snapshot query error: {e:#}"))?;

    let snapshot = Snapshot {
        id: format!("snap_{}", uuid::Uuid::new_v4()),
        source_id: source.id.clone(),
        source_kind: source.kind.as_str().to_string(),
        label: source.label.clone(),
        trigger,
        item_count: items.len() as u32,
        taken_at_ms: chrono::Utc::now().timestamp_millis(),
    };

    let workspace_dir = config.workspace_dir.clone();
    let snap_clone = snapshot.clone();
    let items_clone = items.clone();
    tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            store::insert_snapshot(conn, &snap_clone, &items_clone)?;

            let cutoff = chrono::Utc::now().timestamp_millis()
                - (DEFAULT_RETENTION_DAYS as i64 * 24 * 60 * 60 * 1000);
            store::cleanup_old_snapshots(conn, cutoff, MAX_SNAPSHOTS_PER_SOURCE)?;
            Ok(())
        })
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

    let (to_snap, from_snap, to_items, from_items) = tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            let to_snap = store::get_snapshot(conn, &to_id)?
                .ok_or_else(|| anyhow::anyhow!("snapshot not found: {to_id}"))?;
            let to_items = store::get_snapshot_items(conn, &to_id)?;

            let (from_snap, from_items) = match &from_id {
                Some(fid) => {
                    let s = store::get_snapshot(conn, fid)?
                        .ok_or_else(|| anyhow::anyhow!("snapshot not found: {fid}"))?;
                    if s.source_id != to_snap.source_id {
                        anyhow::bail!(
                            "cross-source diff not allowed: from={} to={}",
                            s.source_id,
                            to_snap.source_id
                        );
                    }
                    let items = store::get_snapshot_items(conn, fid)?;
                    (Some(s), items)
                }
                None => (None, Vec::new()),
            };

            Ok((to_snap, from_snap, to_items, from_items))
        })
    })
    .await
    .map_err(|e| format!("diff join: {e}"))?
    .map_err(|e: anyhow::Error| format!("diff load: {e:#}"))?;

    let from_map: HashMap<&str, &SnapshotItem> =
        from_items.iter().map(|i| (i.item_id.as_str(), i)).collect();
    let to_map: HashMap<&str, &SnapshotItem> =
        to_items.iter().map(|i| (i.item_id.as_str(), i)).collect();

    let mut changes = Vec::new();
    let mut summary = DiffSummary::default();

    // Added + Modified
    for to_item in &to_items {
        match from_map.get(to_item.item_id.as_str()) {
            None => {
                summary.added += 1;
                changes.push(ItemChange {
                    item_id: to_item.item_id.clone(),
                    title: to_item.title.clone(),
                    kind: ChangeKind::Added,
                    old_content_hash: None,
                    new_content_hash: Some(to_item.content_hash.clone()),
                    text_diff: None,
                });
            }
            Some(from_item) => {
                if from_item.content_hash != to_item.content_hash {
                    summary.modified += 1;
                    changes.push(ItemChange {
                        item_id: to_item.item_id.clone(),
                        title: to_item.title.clone(),
                        kind: ChangeKind::Modified,
                        old_content_hash: Some(from_item.content_hash.clone()),
                        new_content_hash: Some(to_item.content_hash.clone()),
                        text_diff: None,
                    });
                } else {
                    summary.unchanged += 1;
                }
            }
        }
    }

    // Removed
    for from_item in &from_items {
        if !to_map.contains_key(from_item.item_id.as_str()) {
            summary.removed += 1;
            changes.push(ItemChange {
                item_id: from_item.item_id.clone(),
                title: from_item.title.clone(),
                kind: ChangeKind::Removed,
                old_content_hash: Some(from_item.content_hash.clone()),
                new_content_hash: None,
                text_diff: None,
            });
        }
    }

    // Compute text diffs for modified items if requested
    if include_text_diff {
        let modified_ids: Vec<String> = changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Modified)
            .map(|c| c.item_id.clone())
            .collect();

        if !modified_ids.is_empty() {
            let source_id = to_snap.source_id.clone();
            let text_diffs =
                compute_text_diffs_from_chunks(config, &source_id, &modified_ids).await;

            for change in &mut changes {
                if change.kind == ChangeKind::Modified {
                    if let Some(diff_text) = text_diffs.get(&change.item_id) {
                        change.text_diff = Some(truncate(diff_text, MAX_TEXT_DIFF_CHARS));
                    }
                }
            }
        }
    }

    Ok(DiffResult {
        source_id: to_snap.source_id.clone(),
        source_kind: to_snap.source_kind.clone(),
        source_label: to_snap.label.clone(),
        from_snapshot_id: from_snap.map(|s| s.id),
        to_snapshot_id: to_snap.id.clone(),
        summary,
        changes,
    })
}

/// Diff current state (latest snapshot) vs previous snapshot for a source.
pub async fn diff_since_last(
    source: &MemorySourceEntry,
    config: &Config,
    include_text_diff: bool,
) -> Result<DiffResult, String> {
    let workspace_dir = config.workspace_dir.clone();
    let source_id = source.id.clone();

    let snapshots = tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            store::latest_snapshots_for_source(conn, &source_id, 2)
        })
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

/// Create a checkpoint that groups the latest snapshot per enabled source.
pub async fn create_checkpoint(label: &str, config: &Config) -> Result<Checkpoint, String> {
    let sources = crate::openhuman::memory_sources::registry::list_sources()
        .await
        .map_err(|e| format!("list sources: {e}"))?;

    let enabled: Vec<_> = sources.into_iter().filter(|s| s.enabled).collect();

    // Take snapshots for any source that doesn't have one yet
    for source in &enabled {
        let workspace_dir = config.workspace_dir.clone();
        let sid = source.id.clone();
        let has_snapshot = tokio::task::spawn_blocking(move || {
            store::with_connection(&workspace_dir, |conn| {
                let snaps = store::latest_snapshots_for_source(conn, &sid, 1)?;
                Ok(!snaps.is_empty())
            })
        })
        .await
        .map_err(|e| format!("checkpoint check join: {e}"))?
        .map_err(|e: anyhow::Error| format!("checkpoint check: {e:#}"))?;

        if !has_snapshot {
            take_snapshot(source, config, SnapshotTrigger::Manual).await?;
        }
    }

    // Collect latest snapshot ID per source
    let workspace_dir = config.workspace_dir.clone();
    let source_ids: Vec<String> = enabled.iter().map(|s| s.id.clone()).collect();
    let snapshot_ids = tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            let mut ids = Vec::new();
            for sid in &source_ids {
                if let Some(snap) = store::latest_snapshots_for_source(conn, sid, 1)?
                    .into_iter()
                    .next()
                {
                    ids.push(snap.id);
                }
            }
            Ok(ids)
        })
    })
    .await
    .map_err(|e| format!("checkpoint gather join: {e}"))?
    .map_err(|e: anyhow::Error| format!("checkpoint gather: {e:#}"))?;

    let checkpoint = Checkpoint {
        id: format!("ckpt_{}", uuid::Uuid::new_v4()),
        label: label.to_string(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
        snapshot_ids: snapshot_ids.clone(),
    };

    let workspace_dir = config.workspace_dir.clone();
    let ckpt_clone = checkpoint.clone();
    tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            store::insert_checkpoint(conn, &ckpt_clone)
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
    let checkpoint = tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            store::get_checkpoint(conn, &ckpt_id)?
                .ok_or_else(|| anyhow::anyhow!("checkpoint not found: {ckpt_id}"))
        })
    })
    .await
    .map_err(|e| format!("checkpoint load join: {e}"))?
    .map_err(|e: anyhow::Error| format!("checkpoint load: {e:#}"))?;

    // For each snapshot in the checkpoint, find the source's latest snapshot
    let workspace_dir = config.workspace_dir.clone();
    let snap_ids = checkpoint.snapshot_ids.clone();
    let snapshot_pairs: Vec<(Snapshot, Option<Snapshot>)> =
        tokio::task::spawn_blocking(move || {
            store::with_connection(&workspace_dir, |conn| {
                let mut pairs = Vec::new();
                for snap_id in &snap_ids {
                    let base_snap = store::get_snapshot(conn, snap_id)?;
                    if let Some(base) = base_snap {
                        let latest = store::latest_snapshots_for_source(conn, &base.source_id, 1)?
                            .into_iter()
                            .next();
                        if let Some(head) = latest {
                            if head.id != base.id {
                                pairs.push((head, Some(base)));
                            }
                            // Same snapshot = no changes, skip
                        }
                    }
                }
                Ok(pairs)
            })
        })
        .await
        .map_err(|e| format!("checkpoint pairs join: {e}"))?
        .map_err(|e: anyhow::Error| format!("checkpoint pairs: {e:#}"))?;

    let mut per_source = Vec::new();
    let mut agg = DiffSummary::default();

    for (head, base) in &snapshot_pairs {
        let diff = compute_diff(
            config,
            base.as_ref().map(|s| s.id.as_str()),
            &head.id,
            include_text_diff,
        )
        .await?;
        agg.added += diff.summary.added;
        agg.removed += diff.summary.removed;
        agg.modified += diff.summary.modified;
        agg.unchanged += diff.summary.unchanged;
        per_source.push(diff);
    }

    Ok(CrossSourceDiff {
        checkpoint_id: Some(checkpoint.id),
        computed_at_ms: chrono::Utc::now().timestamp_millis(),
        summary: agg,
        per_source,
    })
}

/// Delete snapshots older than `days` days.
pub async fn cleanup(config: &Config, older_than_days: u32) -> Result<u64, String> {
    let workspace_dir = config.workspace_dir.clone();
    let cutoff =
        chrono::Utc::now().timestamp_millis() - (older_than_days as i64 * 24 * 60 * 60 * 1000);

    tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            store::cleanup_old_snapshots(conn, cutoff, MAX_SNAPSHOTS_PER_SOURCE)
        })
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

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let mut end = max_chars;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…(truncated)", &s[..end])
    }
}

/// For modified items, read chunk content from the store and compute unified diffs.
async fn compute_text_diffs_from_chunks(
    config: &Config,
    source_id: &str,
    item_ids: &[String],
) -> HashMap<String, String> {
    // Text diffs require reading the current chunk content — this is already
    // in the DB, not an API call. However, we only have the *current* content
    // (the "to" side). The "from" side was overwritten by the new sync.
    // For now, we note this limitation and return empty diffs.
    // A future enhancement could store content snapshots or use the raw files.
    let _ = (config, source_id, item_ids);
    HashMap::new()
}

#[derive(Default)]
struct ItemAccumulator {
    content_parts: Vec<String>,
    max_timestamp_ms: Option<i64>,
    chunk_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let entry = MemorySourceEntry {
            id: "src_abc".into(),
            kind: SourceKind::Folder,
            label: "x".into(),
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
        };
        assert_eq!(source_id_prefix(&entry), "mem_src:src_abc:%");
    }

    #[test]
    fn source_id_prefix_composio() {
        let entry = MemorySourceEntry {
            id: "src_cmp".into(),
            kind: SourceKind::Composio,
            label: "Gmail".into(),
            enabled: true,
            toolkit: Some("gmail".into()),
            connection_id: Some("cmp_1".into()),
            path: None,
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
        };
        assert_eq!(source_id_prefix(&entry), "gmail:%");
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let s = "a".repeat(100);
        let t = truncate(&s, 50);
        assert!(t.len() < 70);
        assert!(t.ends_with("…(truncated)"));
    }

    #[test]
    fn sha256_hex_deterministic() {
        let h1 = sha256_hex(b"hello world");
        let h2 = sha256_hex(b"hello world");
        assert_eq!(h1, h2);
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
    }
}
