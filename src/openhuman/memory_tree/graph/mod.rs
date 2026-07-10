//! Entity co-occurrence graph for E2GraphRAG-style deterministic retrieval.
//!
//! Two pieces:
//! - [`store`] — persistence for `mem_tree_entity_edges`, the undirected
//!   weighted co-occurrence graph built incrementally at ingest time.
//! - [`bfs`] — bounded shortest-path (hop distance) over that graph, used as
//!   the query-time "graph filter" that routes retrieval between the local
//!   (entity-index intersection) and global (dense summary-tree) branches.
//!
//! The graph bridges the entity index and the summary tree without any LLM in
//! the loop: query entities → hop-distance filter → candidate chunk lookup.

pub mod bfs;
pub mod store;

pub use bfs::{pair_distances, PairDistance};
pub use store::{
    clear_edges_for_entities_tx, neighbors, pairs_from_entities, upsert_edges, upsert_edges_tx,
};
