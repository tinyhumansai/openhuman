//! Entities — the `mem_tree_entity_index` table surfaced as a first-class
//! memory_store submodule.
//!
//! The entity index is one of the four primitives memory_store owns
//! (raw / entities / tree / vector + kv). Today its persistence lives in
//! `memory::score::store` because the scorer was the first writer; this
//! module re-exports the read/write surface under the canonical
//! `memory_store::entities::*` path so callers don't have to know about
//! the implementation location.
//!
//! Once the score module finishes splitting (entity persistence vs
//! scoring math), the table-owning code moves here and `memory::score`
//! becomes a pure consumer.
//!
//! ## API
//!
//! | Re-export | Source |
//! | --- | --- |
//! | [`EntityHit`]               | `memory::score::store::EntityHit` |
//! | [`index_entity`]            | `memory::score::store::index_entity` |
//! | [`index_entities`]          | `memory::score::store::index_entities` |
//! | [`lookup_entity`]           | `memory::score::store::lookup_entity` |
//! | [`list_entity_ids_for_node`] | `memory::score::store::list_entity_ids_for_node` |
//! | [`clear_entity_index_for_node`] | `memory::score::store::clear_entity_index_for_node` |
//! | [`count_entity_index`]      | `memory::score::store::count_entity_index` |
//!
//! See [`crate::openhuman::memory_graph`] for the derived co-occurrence
//! query layer built on top of these primitives.

pub use crate::openhuman::memory::score::store::{
    clear_entity_index_for_node, count_entity_index, index_entities, index_entity,
    list_entity_ids_for_node, lookup_entity, EntityHit,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_hit_reexport_is_constructible() {
        let hit = EntityHit {
            entity_id: "person:alice".into(),
            node_id: "chunk-1".into(),
            node_kind: "leaf".into(),
            entity_kind: crate::openhuman::memory::score::extract::EntityKind::Person,
            surface: "Alice".into(),
            score: 1.0,
            timestamp_ms: 123,
            tree_id: Some("tree-1".into()),
            is_user: false,
        };
        assert_eq!(hit.entity_id, "person:alice");
        assert_eq!(hit.node_kind, "leaf");
    }
}
