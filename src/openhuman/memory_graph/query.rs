//! Read-only graph queries derived from `mem_tree_entity_index`.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::params;

use crate::openhuman::config::Config;
use crate::openhuman::memory_graph::types::GraphEdge;
use crate::openhuman::memory_store::chunks::store::with_connection;

/// Return every entity that shares at least one node with `subject_entity`,
/// with a `weight` equal to the number of distinct shared nodes. Sorted by
/// weight DESC, then object id ASC for deterministic output. `limit` caps
/// the result set; `None` defaults to 100.
pub fn co_occurring_entities(
    config: &Config,
    subject_entity: &str,
    limit: Option<usize>,
) -> Result<Vec<GraphEdge>> {
    let cap = limit.unwrap_or(100).min(i64::MAX as usize) as i64;
    with_connection(config, |conn| {
        // SELF JOIN on node_id — every (subject, other) pair counted once
        // per distinct shared node. Excludes self-edges.
        let mut stmt = conn
            .prepare(
                "SELECT b.entity_id AS object, COUNT(DISTINCT a.node_id) AS weight
                   FROM mem_tree_entity_index a
                   JOIN mem_tree_entity_index b ON a.node_id = b.node_id
                  WHERE a.entity_id = ?1
                    AND b.entity_id <> ?1
                  GROUP BY b.entity_id
                  ORDER BY weight DESC, object ASC
                  LIMIT ?2",
            )
            .context("prepare co_occurring_entities")?;
        let rows: Vec<(String, i64)> = stmt
            .query_map(params![subject_entity, cap], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .context("query co_occurring_entities")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect co_occurring_entities rows")?;
        Ok(rows
            .into_iter()
            .map(|(object, weight)| GraphEdge {
                subject: subject_entity.to_string(),
                object,
                weight: weight.max(0) as u32,
            })
            .collect())
    })
}

/// Convenience wrapper around [`co_occurring_entities`] that returns just
/// the neighbor entity ids in weight-descending order.
pub fn neighbors(
    config: &Config,
    subject_entity: &str,
    limit: Option<usize>,
) -> Result<Vec<String>> {
    Ok(co_occurring_entities(config, subject_entity, limit)?
        .into_iter()
        .map(|e| e.object)
        .collect())
}

/// Group the result of [`co_occurring_entities`] by weight. Useful for UIs
/// that want to render strong vs weak relationships separately. Kept here
/// rather than in `types.rs` so it stays a pure derivation helper.
pub fn group_by_weight(edges: Vec<GraphEdge>) -> HashMap<u32, Vec<String>> {
    let mut out: HashMap<u32, Vec<String>> = HashMap::new();
    for e in edges {
        out.entry(e.weight).or_default().push(e.object);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::score::extract::EntityKind;
    use crate::openhuman::memory::score::resolver::CanonicalEntity;
    use crate::openhuman::memory::score::store::index_entity;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn entity(id: &str, kind: EntityKind, surface: &str) -> CanonicalEntity {
        CanonicalEntity {
            canonical_id: id.into(),
            kind,
            surface: surface.into(),
            span_start: 0,
            span_end: surface.len() as u32,
            score: 1.0,
        }
    }

    #[test]
    fn empty_when_no_co_occurrence() {
        let (_tmp, cfg) = test_config();
        let alice = entity(
            "email:alice@example.com",
            EntityKind::Email,
            "alice@example.com",
        );
        index_entity(&cfg, &alice, "leaf-1", "leaf", 100, None).unwrap();
        let neighbors = co_occurring_entities(&cfg, "email:alice@example.com", None).unwrap();
        assert!(neighbors.is_empty());
    }

    #[test]
    fn single_co_occurrence_weight_one() {
        let (_tmp, cfg) = test_config();
        let alice = entity("email:alice@example.com", EntityKind::Email, "a");
        let bob = entity("email:bob@example.com", EntityKind::Email, "b");
        index_entity(&cfg, &alice, "leaf-1", "leaf", 100, None).unwrap();
        index_entity(&cfg, &bob, "leaf-1", "leaf", 100, None).unwrap();
        let edges = co_occurring_entities(&cfg, "email:alice@example.com", None).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].object, "email:bob@example.com");
        assert_eq!(edges[0].weight, 1);
    }

    #[test]
    fn weight_counts_distinct_nodes_not_rows() {
        let (_tmp, cfg) = test_config();
        let alice = entity("email:alice@example.com", EntityKind::Email, "a");
        let bob = entity("email:bob@example.com", EntityKind::Email, "b");
        // Both on leaf-1, leaf-2, leaf-3 -> weight 3.
        for leaf in &["leaf-1", "leaf-2", "leaf-3"] {
            index_entity(&cfg, &alice, leaf, "leaf", 100, None).unwrap();
            index_entity(&cfg, &bob, leaf, "leaf", 100, None).unwrap();
        }
        let edges = co_occurring_entities(&cfg, "email:alice@example.com", None).unwrap();
        assert_eq!(edges[0].weight, 3);
    }

    #[test]
    fn excludes_self_edges() {
        let (_tmp, cfg) = test_config();
        let alice = entity("email:alice@example.com", EntityKind::Email, "a");
        index_entity(&cfg, &alice, "leaf-1", "leaf", 100, None).unwrap();
        index_entity(&cfg, &alice, "leaf-2", "leaf", 100, None).unwrap();
        let edges = co_occurring_entities(&cfg, "email:alice@example.com", None).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn neighbors_returns_ids_in_weight_order() {
        let (_tmp, cfg) = test_config();
        let alice = entity("email:alice@example.com", EntityKind::Email, "a");
        let bob = entity("email:bob@example.com", EntityKind::Email, "b");
        let carol = entity("email:carol@example.com", EntityKind::Email, "c");
        // alice + bob: 2 shared nodes. alice + carol: 1 shared node.
        index_entity(&cfg, &alice, "leaf-1", "leaf", 100, None).unwrap();
        index_entity(&cfg, &bob, "leaf-1", "leaf", 100, None).unwrap();
        index_entity(&cfg, &alice, "leaf-2", "leaf", 100, None).unwrap();
        index_entity(&cfg, &bob, "leaf-2", "leaf", 100, None).unwrap();
        index_entity(&cfg, &alice, "leaf-3", "leaf", 100, None).unwrap();
        index_entity(&cfg, &carol, "leaf-3", "leaf", 100, None).unwrap();
        let ids = neighbors(&cfg, "email:alice@example.com", None).unwrap();
        assert_eq!(
            ids,
            vec![
                "email:bob@example.com".to_string(),
                "email:carol@example.com".to_string(),
            ]
        );
    }
}
