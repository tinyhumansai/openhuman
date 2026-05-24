//! Kind-parameterized tree registry helpers.
//!
//! All three tree flavors (Source, Global, Topic) live in the same
//! `mem_tree_trees` table keyed by [`TreeKind`]. The generic get-or-create
//! dance with UNIQUE-race recovery lives in
//! [`memory_tree::tree::registry::get_or_create_tree`] (logic file — stays
//! in memory_tree). This module hosts the thin per-kind convenience wrappers
//! (formerly spread across `trees_global::registry` and
//! `trees_topic::registry`) so callers have one place to land.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::params;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::with_connection;
use crate::openhuman::memory_store::trees::types::{Tree, TreeKind};
use crate::openhuman::memory_tree::global::GLOBAL_SCOPE;
use crate::openhuman::memory_tree::tree::registry::get_or_create_tree;

/// Return the workspace's singleton global tree, creating it lazily on first
/// call. Safe to call on every ingest; subsequent calls short-circuit to the
/// existing row.
pub fn get_or_create_global_tree(config: &Config) -> Result<Tree> {
    log::debug!("[trees::registry] get_or_create_global_tree");
    get_or_create_tree(config, TreeKind::Global, GLOBAL_SCOPE)
}

/// Look up the topic tree for `entity_id`, or create a new one.
///
/// Callers should NOT use this directly to materialise topic trees eagerly —
/// go through `memory_tree::topic::curator::maybe_spawn_topic_tree` so
/// creation is gated on hotness. Admin / forced-materialisation flows can
/// call this directly (or its alias [`force_create_topic_tree`]).
pub fn get_or_create_topic_tree(config: &Config, entity_id: &str) -> Result<Tree> {
    let entity_kind = entity_id
        .split_once(':')
        .map(|(k, _)| k)
        .unwrap_or("unknown");
    log::debug!(
        "[trees::registry] get_or_create_topic_tree entity_kind={}",
        entity_kind
    );
    get_or_create_tree(config, TreeKind::Topic, entity_id)
}

/// Semantic alias used by the admin "force materialise" path.
pub fn force_create_topic_tree(config: &Config, entity_id: &str) -> Result<Tree> {
    get_or_create_topic_tree(config, entity_id)
}

/// List every tree of the given kind, ordered by creation time ascending.
pub fn list_trees_by_kind(config: &Config, kind: TreeKind) -> Result<Vec<Tree>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, kind, scope, root_id, max_level, status,
                    created_at_ms, last_sealed_at_ms
               FROM mem_tree_trees
              WHERE kind = ?1
              ORDER BY created_at_ms ASC",
        )?;
        let rows = stmt
            .query_map(params![kind.as_str()], row_to_tree_loose)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .with_context(|| format!("failed to list trees of kind {}", kind.as_str()))?;
        Ok(rows)
    })
}

/// Topic-specific alias preserved for call-site readability.
pub fn list_topic_trees(config: &Config) -> Result<Vec<Tree>> {
    list_trees_by_kind(config, TreeKind::Topic)
}

/// Flip a tree's status to `archived`. Idempotent. Existing rows remain
/// queryable; new leaves will not be routed to this tree until it is
/// manually unarchived (which is not currently a primitive).
pub fn archive_tree(config: &Config, tree_id: &str) -> Result<()> {
    use crate::openhuman::memory_store::trees::types::TreeStatus;
    with_connection(config, |conn| {
        let n = conn
            .execute(
                "UPDATE mem_tree_trees
                    SET status = ?1
                  WHERE id = ?2",
                params![TreeStatus::Archived.as_str(), tree_id],
            )
            .with_context(|| format!("failed to archive tree {}", tree_id))?;
        log::debug!("[trees::registry] archive_tree id={} rows={}", tree_id, n);
        Ok(())
    })
}

/// Topic-specific alias preserved for call-site readability.
pub fn archive_topic_tree(config: &Config, tree_id: &str) -> Result<()> {
    archive_tree(config, tree_id)
}

fn row_to_tree_loose(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tree> {
    use crate::openhuman::memory_store::trees::types::TreeStatus;
    use chrono::TimeZone;
    let id: String = row.get(0)?;
    let kind_s: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let root_id: Option<String> = row.get(3)?;
    let max_level: i64 = row.get(4)?;
    let status_s: String = row.get(5)?;
    let created_ms: i64 = row.get(6)?;
    let last_sealed_ms: Option<i64> = row.get(7)?;
    let kind = TreeKind::parse(&kind_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })?;
    let status = TreeStatus::parse(&status_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })?;
    Ok(Tree {
        id,
        kind,
        scope,
        root_id,
        max_level: max_level.max(0) as u32,
        status,
        created_at: Utc.timestamp_millis_opt(created_ms).unwrap(),
        last_sealed_at: last_sealed_ms.and_then(|ms| Utc.timestamp_millis_opt(ms).single()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_store::trees::store::insert_tree;
    use crate::openhuman::memory_store::trees::types::TreeStatus;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn sample_tree(id: &str, kind: TreeKind, scope: &str) -> Tree {
        Tree {
            id: id.into(),
            kind,
            scope: scope.into(),
            root_id: Some("root-1".into()),
            max_level: 2,
            status: TreeStatus::Active,
            created_at: Utc::now(),
            last_sealed_at: None,
        }
    }

    #[test]
    fn list_trees_by_kind_returns_only_requested_kind() {
        let (_tmp, cfg) = test_config();
        insert_tree(
            &cfg,
            &sample_tree("source-1", TreeKind::Source, "chat:slack:#eng"),
        )
        .unwrap();
        insert_tree(
            &cfg,
            &sample_tree("topic-1", TreeKind::Topic, "person:alice"),
        )
        .unwrap();
        insert_tree(
            &cfg,
            &sample_tree("source-2", TreeKind::Source, "chat:discord:#ops"),
        )
        .unwrap();

        let source_ids: Vec<String> = list_trees_by_kind(&cfg, TreeKind::Source)
            .unwrap()
            .into_iter()
            .map(|tree| tree.id)
            .collect();
        assert_eq!(
            source_ids,
            vec!["source-1".to_string(), "source-2".to_string()]
        );

        let topic_ids: Vec<String> = list_topic_trees(&cfg)
            .unwrap()
            .into_iter()
            .map(|tree| tree.id)
            .collect();
        assert_eq!(topic_ids, vec!["topic-1".to_string()]);
    }

    #[test]
    fn archive_tree_flips_status_to_archived() {
        let (_tmp, cfg) = test_config();
        let tree = sample_tree("topic-1", TreeKind::Topic, "person:alice");
        insert_tree(&cfg, &tree).unwrap();

        archive_tree(&cfg, "topic-1").unwrap();

        let archived = list_topic_trees(&cfg).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].status, TreeStatus::Archived);
    }

    #[test]
    fn get_or_create_global_tree_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let first = get_or_create_global_tree(&cfg).unwrap();
        let second = get_or_create_global_tree(&cfg).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.kind, TreeKind::Global);
        assert_eq!(first.scope, GLOBAL_SCOPE);
    }
}
