//! Bounded shortest-path (hop distance) over the entity co-occurrence graph.
//!
//! This is the E2GraphRAG "graph filter" step: given the entities extracted
//! from a query, decide which *pairs* are semantically related by checking
//! whether they sit within `h` hops of each other in the co-occurrence graph.
//! Pairs within range route retrieval to the *local* (index-intersection)
//! branch; an empty result routes to the *global* (dense-tree) branch.
//!
//! Everything here is pure code — no LLM. Lookups go through
//! [`super::store::neighbors`], cached per query so a node is expanded at most
//! once even when it lies between several query-entity pairs.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::graph::store;

/// One related query-entity pair and the hop distance between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairDistance {
    pub a: String,
    pub b: String,
    pub dist: u32,
}

/// Per-query neighbour cache so each entity's adjacency is read from SQLite at
/// most once across all pair BFS runs.
struct NeighborCache<'a> {
    config: &'a Config,
    cache: HashMap<String, Vec<String>>,
}

impl<'a> NeighborCache<'a> {
    fn new(config: &'a Config) -> Self {
        Self {
            config,
            cache: HashMap::new(),
        }
    }

    fn neighbors(&mut self, node: &str) -> Result<&[String]> {
        if !self.cache.contains_key(node) {
            let adj = store::neighbors(self.config, node)?
                .into_iter()
                .map(|(other, _weight)| other)
                .collect();
            self.cache.insert(node.to_string(), adj);
        }
        // Safe: just inserted if missing.
        Ok(self.cache.get(node).unwrap())
    }
}

/// Compute hop distances between every unordered pair of `entity_ids`, keeping
/// only pairs reachable within `max_h` hops. `max_h == 0` short-circuits to an
/// empty result (no pair can be 0 hops apart — distinct entities are ≥1 hop).
///
/// A breadth-first search runs from each entity, stopping once it has either
/// found all its peers in the query set or exhausted the `max_h` frontier.
/// Distance 1 means the two entities co-occurred directly in some chunk.
pub fn pair_distances(
    config: &Config,
    entity_ids: &[String],
    max_h: u32,
) -> Result<Vec<PairDistance>> {
    if max_h == 0 || entity_ids.len() < 2 {
        return Ok(Vec::new());
    }
    // Deduplicate while preserving a stable canonical order so output pairs are
    // deterministic (a < b).
    let mut unique: Vec<String> = entity_ids.to_vec();
    unique.sort();
    unique.dedup();
    if unique.len() < 2 {
        return Ok(Vec::new());
    }
    let targets: HashSet<&str> = unique.iter().map(|s| s.as_str()).collect();

    let mut cache = NeighborCache::new(config);
    let mut out: Vec<PairDistance> = Vec::new();
    // Track pairs already recorded so a BFS from both endpoints doesn't emit
    // the same pair twice. Keyed canonically (a < b).
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for src in &unique {
        // Peers we still need to reach from `src`; once empty we can stop the
        // BFS early instead of draining the whole frontier (latency then
        // scales with unresolved pairs, not graph size).
        let mut remaining: HashSet<&str> = targets
            .iter()
            .copied()
            .filter(|t| *t != src.as_str())
            .collect();
        if remaining.is_empty() {
            continue;
        }
        // BFS frontier from `src` bounded to `max_h`.
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(src.clone());
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        queue.push_back((src.clone(), 0));

        while let Some((node, dist)) = queue.pop_front() {
            if dist >= max_h || remaining.is_empty() {
                continue;
            }
            let neighbors: Vec<String> = cache.neighbors(&node)?.to_vec();
            for nb in neighbors {
                if !visited.insert(nb.clone()) {
                    continue;
                }
                let nd = dist + 1;
                // Record a pair the moment we reach another query target. BFS
                // guarantees `nd` is the shortest distance.
                if nb.as_str() != src.as_str() && remaining.remove(nb.as_str()) {
                    let (a, b) = if src.as_str() < nb.as_str() {
                        (src.clone(), nb.clone())
                    } else {
                        (nb.clone(), src.clone())
                    };
                    if seen.insert((a.clone(), b.clone())) {
                        out.push(PairDistance { a, b, dist: nd });
                    }
                }
                queue.push_back((nb, nd));
            }
        }
    }

    out.sort_by(|x, y| {
        x.dist
            .cmp(&y.dist)
            .then_with(|| x.a.cmp(&y.a))
            .then_with(|| x.b.cmp(&y.b))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_tree::graph::store::{pairs_from_entities, upsert_edges};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    /// Seed a path graph alice — bob — carol (two chunks).
    fn seed_path(cfg: &Config) {
        upsert_edges(
            cfg,
            &pairs_from_entities(&["person:alice".into(), "person:bob".into()]),
            1_000,
        )
        .unwrap();
        upsert_edges(
            cfg,
            &pairs_from_entities(&["person:bob".into(), "person:carol".into()]),
            1_000,
        )
        .unwrap();
    }

    #[test]
    fn direct_cooccurrence_is_distance_one() {
        let (_tmp, cfg) = test_config();
        seed_path(&cfg);
        let pairs = pair_distances(&cfg, &["person:alice".into(), "person:bob".into()], 2).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].dist, 1);
        assert_eq!(pairs[0].a, "person:alice");
        assert_eq!(pairs[0].b, "person:bob");
    }

    #[test]
    fn two_hop_pair_found_within_h2_not_h1() {
        let (_tmp, cfg) = test_config();
        seed_path(&cfg);
        let ids = vec!["person:alice".to_string(), "person:carol".to_string()];

        let h1 = pair_distances(&cfg, &ids, 1).unwrap();
        assert!(
            h1.is_empty(),
            "alice-carol is 2 hops apart; h=1 finds nothing"
        );

        let h2 = pair_distances(&cfg, &ids, 2).unwrap();
        assert_eq!(h2.len(), 1);
        assert_eq!(h2[0].dist, 2);
    }

    #[test]
    fn disconnected_entities_yield_no_pairs() {
        let (_tmp, cfg) = test_config();
        seed_path(&cfg);
        // dave is not in the graph at all.
        let pairs =
            pair_distances(&cfg, &["person:alice".into(), "person:dave".into()], 3).unwrap();
        assert!(pairs.is_empty());
    }

    #[test]
    fn single_entity_yields_no_pairs() {
        let (_tmp, cfg) = test_config();
        seed_path(&cfg);
        let pairs = pair_distances(&cfg, &["person:alice".into()], 2).unwrap();
        assert!(pairs.is_empty());
    }
}
