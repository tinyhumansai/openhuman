//! The host implementation of the crate diff engine's chunk-source seam.
//!
//! `tinycortex::memory::diff::DiffEngine` is generic over a
//! [`SnapshotItemSource`](tinycortex::memory::diff::SnapshotItemSource): during
//! `take_snapshot` (directly, and transitively from `create_checkpoint` for any
//! source lacking a baseline) it asks the source for a source's already-ingested
//! items rather than re-calling readers. In OpenHuman that data lives in
//! `mem_tree_chunks`, so [`ChunkStoreItemSource`] answers the seam by querying
//! the chunk store — the exact query the host `take_snapshot` used before the
//! engine was ported to the crate (group by item id, concatenate chunk bodies in
//! `seq_in_source` order, sort by item id).
//!
//! ## Why the adapter holds a prefix map
//!
//! The crate calls `items_for_source(source_id)` with the *logical* source id,
//! but the host chunk `source_id LIKE` prefix is kind-dependent — Composio
//! sources key their chunks by `<toolkit>:%`, not `mem_src:<id>:%`, and the
//! toolkit is not derivable from the logical id alone. The adapter is therefore
//! built from the full [`MemorySourceEntry`] list (which carries `toolkit`) and
//! resolves each id → prefix up front.

use std::collections::HashMap;

use tinycortex::memory::diff::{extract_item_id, SnapshotItem, SnapshotItemSource};

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};

/// Host [`SnapshotItemSource`] backed by `mem_tree_chunks`.
///
/// Construct with [`single`](Self::single) for the per-source `take_snapshot`
/// path, [`for_sources`](Self::for_sources) for `create_checkpoint` (which may
/// baseline several sources), or [`read_only`](Self::read_only) for operations
/// that never materialise items (diff/list/cleanup) and just need *some* source
/// to satisfy the engine's type parameter.
pub struct ChunkStoreItemSource {
    config: Config,
    /// Logical source id → chunk `source_id LIKE` prefix.
    prefixes: HashMap<String, String>,
}

impl ChunkStoreItemSource {
    /// Adapter that can materialise items for any of `sources`.
    pub fn for_sources(config: Config, sources: &[MemorySourceEntry]) -> Self {
        let prefixes = sources
            .iter()
            .map(|s| (s.id.clone(), source_id_prefix(s)))
            .collect();
        Self { config, prefixes }
    }

    /// Adapter scoped to a single source (the common `take_snapshot` path).
    pub fn single(config: Config, source: &MemorySourceEntry) -> Self {
        let mut prefixes = HashMap::new();
        prefixes.insert(source.id.clone(), source_id_prefix(source));
        Self { config, prefixes }
    }

    /// Adapter that never yields items — for read-only ops (`compute_diff`,
    /// `diff_since_*`, `mark_read`, `diff_since_checkpoint`, `cleanup`) whose
    /// engine calls only touch the ledger. `items_for_source` always returns
    /// empty; it is never invoked on these paths.
    pub fn read_only(config: Config) -> Self {
        Self {
            config,
            prefixes: HashMap::new(),
        }
    }
}

impl SnapshotItemSource for ChunkStoreItemSource {
    fn items_for_source(&self, source_id: &str) -> Vec<SnapshotItem> {
        let Some(prefix) = self.prefixes.get(source_id) else {
            return Vec::new();
        };

        let result =
            crate::openhuman::memory_store::chunks::store::with_connection(&self.config, |conn| {
                let mut stmt = conn.prepare(
                    "SELECT source_id, content \
                     FROM mem_tree_chunks \
                     WHERE source_id LIKE ?1 \
                     ORDER BY source_id, seq_in_source",
                )?;

                let mut groups: HashMap<String, Vec<String>> = HashMap::new();
                let rows = stmt.query_map([prefix], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (composite_source_id, content) = row?;
                    let item_id = extract_item_id(&composite_source_id);
                    groups.entry(item_id).or_default().push(content);
                }

                let mut items: Vec<SnapshotItem> = groups
                    .into_iter()
                    .map(|(item_id, parts)| SnapshotItem {
                        item_id,
                        content: parts.join(""),
                    })
                    .collect();
                items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
                Ok(items)
            });

        match result {
            Ok(items) => items,
            Err(e) => {
                // The crate seam has no error channel. A chunk-store read
                // failure here yields an empty snapshot (every item reads as
                // removed for that one diff) rather than a propagated error —
                // but the ledger is a derived, rebuildable view, so the next
                // successful snapshot restores the true state. Log loudly.
                tracing::error!(
                    source_id = %source_id,
                    error = %format!("{e:#}"),
                    "[memory_diff] chunk item-source query failed; snapshot will see no items"
                );
                Vec::new()
            }
        }
    }
}

/// Build the `source_id LIKE` prefix that matches chunks belonging to a source.
/// Mirrors `memory_sources::status::source_id_prefix`.
pub(crate) fn source_id_prefix(source: &MemorySourceEntry) -> String {
    match source.kind {
        SourceKind::Composio => source
            .toolkit
            .as_deref()
            .map(|t| format!("{t}:%"))
            .unwrap_or_else(|| "__no_toolkit__:%".to_string()),
        _ => format!("mem_src:{}:%", source.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn source_id_prefix_composio_without_toolkit() {
        let mut entry = folder_source("src_cmp");
        entry.kind = SourceKind::Composio;
        entry.toolkit = None;
        assert_eq!(source_id_prefix(&entry), "__no_toolkit__:%");
    }

    #[test]
    fn read_only_adapter_never_yields_items() {
        let source = ChunkStoreItemSource::read_only(Config::default());
        assert!(source.items_for_source("anything").is_empty());
    }
}
