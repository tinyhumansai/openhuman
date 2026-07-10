//! Persistence for the entity co-occurrence graph (`mem_tree_entity_edges`).
//!
//! The edge table is an undirected weighted graph: each row is one unordered
//! pair of canonical entity ids that co-occurred inside the same chunk, with
//! `weight` accumulating the number of co-occurrences. Canonical ordering
//! (`entity_a < entity_b`) keeps a pair on a single row; neighbour lookups
//! union both columns so a query entity matches regardless of which side it
//! was stored on.
//!
//! The schema is declared in `memory_store/chunks/store.rs::SCHEMA`; this file
//! only owns the CRUD operations. Writes happen inside the same transaction
//! that indexes a chunk's entities (see `score::persist_score_tx`) so the
//! graph never diverges from `mem_tree_entity_index`.

use anyhow::Result;
use rusqlite::{params, Transaction};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::with_connection;

/// Order a pair canonically so `(a, b)` and `(b, a)` collapse onto one row.
/// Returns `None` for a self-pair (`a == b`) — an entity never edges itself.
fn order_pair<'a>(a: &'a str, b: &'a str) -> Option<(&'a str, &'a str)> {
    use std::cmp::Ordering;
    match a.cmp(b) {
        Ordering::Less => Some((a, b)),
        Ordering::Greater => Some((b, a)),
        Ordering::Equal => None,
    }
}

/// Emit every unordered pair from a set of canonical entity ids. Deduplicates
/// the input first (a chunk can mention the same entity in several spans) so a
/// repeated id doesn't inflate a single chunk's contribution to one edge.
pub fn pairs_from_entities(entity_ids: &[String]) -> Vec<(String, String)> {
    use std::collections::BTreeSet;
    let unique: Vec<&String> = entity_ids
        .iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut out = Vec::new();
    for i in 0..unique.len() {
        for j in (i + 1)..unique.len() {
            if let Some((a, b)) = order_pair(unique[i], unique[j]) {
                out.push((a.to_string(), b.to_string()));
            }
        }
    }
    out
}

/// Upsert a batch of co-occurrence pairs inside an existing transaction,
/// incrementing `weight` for pairs that already exist. Idempotent on the
/// `(entity_a, entity_b)` primary key — re-running adds to the weight, which
/// is the intended accumulation semantics (more co-occurrences = stronger
/// edge). `pairs` are expected to already be canonically ordered via
/// [`pairs_from_entities`].
pub fn upsert_edges_tx(
    tx: &Transaction<'_>,
    pairs: &[(String, String)],
    timestamp_ms: i64,
) -> Result<usize> {
    // Defensively canonicalise (`a < b`) and dedup here too, so a caller that
    // passes reversed or duplicate pairs can never split one edge across two
    // rows (`a,b` + `b,a`) or double-count a single chunk's contribution.
    // `pairs_from_entities` already does this, but the table invariant must not
    // depend on callers getting it right.
    use std::collections::BTreeSet;
    let canonical: BTreeSet<(String, String)> = pairs
        .iter()
        .filter_map(|(a, b)| order_pair(a, b).map(|(x, y)| (x.to_string(), y.to_string())))
        .collect();
    if canonical.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT INTO mem_tree_entity_edges (entity_a, entity_b, weight, updated_ms)
             VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(entity_a, entity_b)
             DO UPDATE SET weight = weight + 1, updated_ms = ?3",
    )?;
    for (a, b) in &canonical {
        stmt.execute(params![a, b, timestamp_ms])?;
    }
    Ok(canonical.len())
}

/// Convenience wrapper that opens its own connection + transaction. Prefer
/// [`upsert_edges_tx`] when an enclosing transaction already exists (the
/// ingest hook does), so the graph write commits atomically with the entity
/// index write.
pub fn upsert_edges(
    config: &Config,
    pairs: &[(String, String)],
    timestamp_ms: i64,
) -> Result<usize> {
    if pairs.is_empty() {
        return Ok(0);
    }
    with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;
        let n = upsert_edges_tx(&tx, pairs, timestamp_ms)?;
        tx.commit()?;
        Ok(n)
    })
}

/// Return the immediate neighbours of `entity_id` with their edge weights.
/// Unions both columns so the lookup is symmetric. Ordered by weight DESC so
/// the strongest associations come first (callers can cap traversal breadth).
pub fn neighbors(config: &Config, entity_id: &str) -> Result<Vec<(String, i64)>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT entity_b AS other, weight FROM mem_tree_entity_edges WHERE entity_a = ?1
             UNION ALL
             SELECT entity_a AS other, weight FROM mem_tree_entity_edges WHERE entity_b = ?1
             ORDER BY weight DESC",
        )?;
        let rows = stmt
            .query_map(params![entity_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Remove every edge touching any of `entity_ids`. Used before re-indexing a
/// re-scored chunk so co-occurrences that no longer hold don't leak through as
/// stale edges (mirrors `clear_entity_index_for_node`). Note this is coarse —
/// it drops the entity's edges to *all* peers, not just the ones from this
/// chunk; the subsequent re-index rebuilds the surviving edges from the
/// chunk's current entities. Edges contributed by other chunks are recovered
/// on their next re-score; for the common append-only ingest path this branch
/// is not exercised.
pub fn clear_edges_for_entities_tx(tx: &Transaction<'_>, entity_ids: &[String]) -> Result<usize> {
    if entity_ids.is_empty() {
        return Ok(0);
    }
    let mut removed = 0;
    let mut stmt =
        tx.prepare("DELETE FROM mem_tree_entity_edges WHERE entity_a = ?1 OR entity_b = ?1")?;
    for id in entity_ids {
        removed += stmt.execute(params![id])?;
    }
    Ok(removed)
}

/// Count edge rows (for tests / diagnostics).
pub fn count_edges(config: &Config) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM mem_tree_entity_edges", [], |r| {
            r.get(0)
        })?;
        Ok(n.max(0) as u64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    #[test]
    fn pairs_are_canonically_ordered_and_deduped() {
        let pairs = pairs_from_entities(&[
            "person:bob".into(),
            "person:alice".into(),
            "person:alice".into(), // dup — must not double an edge
        ]);
        assert_eq!(pairs, vec![("person:alice".into(), "person:bob".into())]);
    }

    #[test]
    fn self_pairs_are_skipped() {
        let pairs = pairs_from_entities(&["x:1".into(), "x:1".into()]);
        assert!(pairs.is_empty());
    }

    #[test]
    fn upsert_increments_weight_and_neighbors_is_symmetric() {
        let (_tmp, cfg) = test_config();
        let p = vec![("person:alice".to_string(), "person:bob".to_string())];
        upsert_edges(&cfg, &p, 1_000).unwrap();
        upsert_edges(&cfg, &p, 2_000).unwrap();

        // Symmetric lookup from either endpoint.
        let from_alice = neighbors(&cfg, "person:alice").unwrap();
        assert_eq!(from_alice, vec![("person:bob".to_string(), 2)]);
        let from_bob = neighbors(&cfg, "person:bob").unwrap();
        assert_eq!(from_bob, vec![("person:alice".to_string(), 2)]);
        assert_eq!(count_edges(&cfg).unwrap(), 1);
    }

    #[test]
    fn clear_edges_removes_all_touching() {
        let (_tmp, cfg) = test_config();
        upsert_edges(
            &cfg,
            &pairs_from_entities(&[
                "person:alice".into(),
                "person:bob".into(),
                "topic:phoenix".into(),
            ]),
            1_000,
        )
        .unwrap();
        assert_eq!(count_edges(&cfg).unwrap(), 3);

        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            let n = clear_edges_for_entities_tx(&tx, &["person:alice".into()]).unwrap();
            tx.commit()?;
            // alice-bob and alice-phoenix removed; bob-phoenix survives.
            assert_eq!(n, 2);
            Ok(())
        })
        .unwrap();
        assert_eq!(count_edges(&cfg).unwrap(), 1);
        assert!(neighbors(&cfg, "person:alice").unwrap().is_empty());
    }
}
