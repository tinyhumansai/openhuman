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
/// Upper bound on per-item content persisted into a snapshot, so the "from"
/// side of a text diff survives the next sync overwriting the live chunk
/// store. Items larger than this skip content capture (`content = None`);
/// hash-based add/remove/modify detection still works for them.
const MAX_SNAPSHOT_CONTENT_BYTES: usize = 64 * 1024;

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
                    let title = derive_title(&item_id, &concat);
                    // Persist bounded content so a future diff has both sides.
                    let content = if concat.len() <= MAX_SNAPSHOT_CONTENT_BYTES {
                        Some(concat)
                    } else {
                        None
                    };
                    SnapshotItem {
                        item_id,
                        title,
                        content_hash: hash,
                        content,
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
            let text_diffs = compute_text_diffs_from_snapshots(&from_map, &to_map, &modified_ids);

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

/// Diff a source's latest snapshot against its read marker — i.e. everything
/// that changed since the agent last *read* this source's diff.
///
/// When `commit` is true, the read marker is advanced to the head snapshot
/// after the diff is computed, so a subsequent call returns only newer
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
    // marker points at a snapshot that has since been cleaned up, treat it as
    // unread (base = None) rather than erroring.
    let (head, base_id) = tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            let head = store::latest_snapshots_for_source(conn, &source_id, 1)?
                .into_iter()
                .next();
            let marker = store::get_read_marker(conn, &source_id)?;
            let base_id = match marker {
                Some(snap_id) if store::get_snapshot(conn, &snap_id)?.is_some() => Some(snap_id),
                _ => None,
            };
            Ok((head, base_id))
        })
    })
    .await
    .map_err(|e| format!("diff_since_read join: {e}"))?
    .map_err(|e: anyhow::Error| format!("diff_since_read: {e:#}"))?;

    let head = head.ok_or_else(|| "no snapshots found for this source".to_string())?;

    // Marker already at head → nothing new since last read.
    let from_id = match &base_id {
        Some(id) if *id == head.id => Some(head.id.as_str()),
        Some(id) => Some(id.as_str()),
        None => None,
    };

    let diff = compute_diff(config, from_id, &head.id, include_text_diff).await?;

    if commit {
        let workspace_dir = config.workspace_dir.clone();
        let source_id = source.id.clone();
        let head_id = head.id.clone();
        let now_ms = chrono::Utc::now().timestamp_millis();
        tokio::task::spawn_blocking(move || {
            store::with_connection(&workspace_dir, |conn| {
                store::upsert_read_marker(conn, &source_id, &head_id, now_ms)
            })
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
    let (marked, snapshot_ids) = tokio::task::spawn_blocking(move || {
        store::with_connection(&workspace_dir, |conn| {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let mut count = 0u64;
            let mut snapshot_ids = Vec::new();
            for sid in &ids_for_blocking {
                if let Some(head) = store::latest_snapshots_for_source(conn, sid, 1)?
                    .into_iter()
                    .next()
                {
                    store::upsert_read_marker(conn, sid, &head.id, now_ms)?;
                    snapshot_ids.push(head.id);
                    count += 1;
                }
            }
            Ok((count, snapshot_ids))
        })
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

/// Derive a human-readable title for an item from its content.
///
/// Uses the first non-empty line (a Markdown heading marker is stripped),
/// trimmed and bounded. Falls back to the item id when no usable line exists
/// (e.g. binary or empty content) so the diff output is never blank.
fn derive_title(item_id: &str, content: &str) -> String {
    let first_line = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('#').trim());

    match first_line {
        Some(l) if !l.is_empty() => truncate(l, 120),
        _ => item_id.to_string(),
    }
}

/// Compute unified text diffs for modified items from the content stored in
/// the two snapshots being compared. Both sides are read from the diff DB
/// (bounded content captured at snapshot time), so this works even after the
/// live chunk store has been overwritten by a later sync. Items whose content
/// was too large to capture (`content = None` on either side) are skipped.
fn compute_text_diffs_from_snapshots(
    from_items: &HashMap<&str, &SnapshotItem>,
    to_items: &HashMap<&str, &SnapshotItem>,
    item_ids: &[String],
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for item_id in item_ids {
        let (Some(from), Some(to)) = (
            from_items.get(item_id.as_str()),
            to_items.get(item_id.as_str()),
        ) else {
            continue;
        };
        let (Some(old), Some(new)) = (from.content.as_deref(), to.content.as_deref()) else {
            continue;
        };
        let diff = similar::TextDiff::from_lines(old, new);
        let unified = diff
            .unified_diff()
            .context_radius(3)
            .header("before", "after")
            .to_string();
        if !unified.trim().is_empty() {
            out.insert(item_id.clone(), unified);
        }
    }
    out
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

    #[test]
    fn derive_title_uses_first_nonempty_line() {
        assert_eq!(derive_title("file.md", "# Heading\nbody"), "Heading");
        assert_eq!(
            derive_title("file.md", "\n\n  Plain title  \nmore"),
            "Plain title"
        );
    }

    #[test]
    fn derive_title_falls_back_to_item_id() {
        assert_eq!(derive_title("doc_42", ""), "doc_42");
        assert_eq!(derive_title("doc_42", "   \n  "), "doc_42");
    }

    // ── Integration-style ops tests over a temp diff.db ───────────────────

    fn test_config() -> Config {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        // Leak the tempdir so the path stays valid for the test's lifetime.
        std::mem::forget(dir);
        config
    }

    fn item(item_id: &str, hash: &str, content: &str) -> SnapshotItem {
        SnapshotItem {
            item_id: item_id.to_string(),
            title: item_id.to_string(),
            content_hash: hash.to_string(),
            content: Some(content.to_string()),
            timestamp_ms: Some(1000),
            chunk_count: 1,
        }
    }

    fn seed(config: &Config, id: &str, source_id: &str, taken_at_ms: i64, items: &[SnapshotItem]) {
        let snap = Snapshot {
            id: id.to_string(),
            source_id: source_id.to_string(),
            source_kind: "folder".to_string(),
            label: "Docs".to_string(),
            trigger: SnapshotTrigger::Auto,
            item_count: items.len() as u32,
            taken_at_ms,
        };
        store::with_connection(&config.workspace_dir, |conn| {
            store::insert_snapshot(conn, &snap, items)
        })
        .unwrap();
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

    #[tokio::test]
    async fn compute_diff_detects_added_modified_removed() {
        let config = test_config();
        // from: a(h1), b(h2), c(h3)
        seed(
            &config,
            "snap_from",
            "src_a",
            1000,
            &[
                item("a", "h1", "alpha"),
                item("b", "h2", "beta"),
                item("c", "h3", "gamma"),
            ],
        );
        // to: a(h1, unchanged), b(h2b, modified), c removed, d(h4, added)
        seed(
            &config,
            "snap_to",
            "src_a",
            2000,
            &[
                item("a", "h1", "alpha"),
                item("b", "h2b", "beta v2"),
                item("d", "h4", "delta"),
            ],
        );

        let diff = compute_diff(&config, Some("snap_from"), "snap_to", false)
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
        seed(&config, "snap_to", "src_a", 1000, &[item("a", "h1", "x")]);
        let diff = compute_diff(&config, None, "snap_to", false).await.unwrap();
        assert_eq!(diff.summary.added, 1);
        assert_eq!(diff.from_snapshot_id, None);
    }

    #[tokio::test]
    async fn compute_diff_rejects_cross_source() {
        let config = test_config();
        seed(&config, "from_a", "src_a", 1000, &[]);
        seed(&config, "to_b", "src_b", 2000, &[]);
        let err = compute_diff(&config, Some("from_a"), "to_b", false)
            .await
            .unwrap_err();
        assert!(err.contains("cross-source"), "got: {err}");
    }

    #[tokio::test]
    async fn compute_diff_text_diff_only_when_requested() {
        let config = test_config();
        seed(
            &config,
            "f",
            "src_a",
            1000,
            &[item("a", "h1", "line one\nline two\n")],
        );
        seed(
            &config,
            "t",
            "src_a",
            2000,
            &[item("a", "h2", "line one\nline TWO changed\n")],
        );

        let without = compute_diff(&config, Some("f"), "t", false).await.unwrap();
        assert!(without.changes[0].text_diff.is_none());

        let with = compute_diff(&config, Some("f"), "t", true).await.unwrap();
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
        seed(&config, "s1", "src_a", 1000, &[item("a", "h1", "x")]);
        let one = diff_since_last(&source, &config, false).await.unwrap();
        assert_eq!(one.summary.added, 1);

        // 2 snapshots → diff latest vs previous
        seed(
            &config,
            "s2",
            "src_a",
            2000,
            &[item("a", "h1", "x"), item("b", "h2", "y")],
        );
        let two = diff_since_last(&source, &config, false).await.unwrap();
        assert_eq!(two.summary.added, 1, "b is new in s2");
        assert_eq!(two.summary.unchanged, 1, "a unchanged");
    }

    #[tokio::test]
    async fn diff_since_read_commits_marker_and_returns_only_new_changes() {
        let config = test_config();
        let source = folder_source("src_a");

        seed(&config, "s1", "src_a", 1000, &[item("a", "h1", "x")]);

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
        seed(
            &config,
            "s2",
            "src_a",
            2000,
            &[item("a", "h1", "x"), item("b", "h2", "y")],
        );
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
        seed(&config, "s1", "src_a", 1000, &[item("a", "h1", "x")]);

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
        seed(&config, "s1", "src_a", 1000, &[item("a", "h1", "x")]);

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
        seed(&config, "a1", "src_a", 1000, &[item("a", "h1", "x")]);
        seed(&config, "b1", "src_b", 1000, &[item("b", "h1", "y")]);
        let ckpt = Checkpoint {
            id: "ckpt_1".to_string(),
            label: "base".to_string(),
            created_at_ms: 1500,
            snapshot_ids: vec!["a1".to_string(), "b1".to_string()],
        };
        store::with_connection(&config.workspace_dir, |conn| {
            store::insert_checkpoint(conn, &ckpt)
        })
        .unwrap();

        // src_a gets a new head with a modification; src_b unchanged (no new head).
        seed(&config, "a2", "src_a", 2000, &[item("a", "h2", "x v2")]);

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
