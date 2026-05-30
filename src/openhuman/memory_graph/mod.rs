//! Memory graph — placeholder over the existing tree entity index.
//!
//! The premise: a separate triple store (`unified::graph`) is redundant
//! when every chunk already lands an entity row in `mem_tree_entity_index`.
//! The graph IS the tree mapped out — two entities co-occurring on the
//! same leaf form an edge.
//!
//! This module derives those edges on demand instead of writing a parallel
//! storage table. It's a placeholder while the existing `unified::graph`
//! callers (ingestion's LLM-extracted triples + the public client RPC)
//! get migrated or retired; the LLM-extracted (subject, predicate, object)
//! triple surface is intentionally not covered here.
//!
//! ## API
//!
//! - [`co_occurring_entities`] — for a subject entity, return every other
//!   entity that has appeared on the same node, with a co-occurrence
//!   count.
//! - [`neighbors`] — convenience: just the entity ids, no counts.
//!
//! ## Layer rules
//!
//! - Reads from `mem_tree_entity_index` via
//!   `memory_store::chunks::store::with_connection`. No writes.
//! - No new tables, no new schema. Anything you can't derive from the
//!   entity index is intentionally out of scope here.

pub mod query;
pub mod types;

pub use query::{co_occurring_entities, neighbors};
pub use types::GraphEdge;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_edge_reexport_is_constructible() {
        let edge = GraphEdge {
            subject: "person:alice".into(),
            object: "topic:phoenix".into(),
            weight: 2,
        };
        assert_eq!(edge.weight, 2);
        assert_eq!(edge.subject, "person:alice");
    }
}
