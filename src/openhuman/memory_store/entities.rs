//! Host adapters for tinycortex's entity occurrence index.

use std::sync::Arc;

use anyhow::Result;
use tinycortex::memory::store::entity_index::{
    CanonicalEntity, EntityIndex, EntityKind, SelfIdentity,
};

use crate::openhuman::composio::providers::profile::{is_self_identity_any_toolkit, IdentityKind};
use crate::openhuman::config::Config;
use crate::openhuman::tinycortex::memory_config_from;

pub use tinycortex::memory::store::entity_index::EntityHit;

#[derive(Debug)]
struct HostSelfIdentity;

impl SelfIdentity for HostSelfIdentity {
    fn is_self(&self, kind: EntityKind, surface: &str) -> bool {
        let identity_kind = match kind {
            EntityKind::Email => IdentityKind::Email,
            EntityKind::Handle => IdentityKind::Handle,
            _ => return false,
        };
        is_self_identity_any_toolkit(identity_kind, surface)
    }
}

fn index(config: &Config) -> Result<EntityIndex> {
    let memory = memory_config_from(config, config.workspace_dir.clone());
    let connection = tinycortex::memory::chunks::shared_connection(&memory)?;
    EntityIndex::from_shared_connection(connection, Arc::new(HostSelfIdentity))
}

pub(crate) fn host_self_identity() -> Arc<dyn SelfIdentity> {
    Arc::new(HostSelfIdentity)
}

pub fn index_entity(
    config: &Config,
    entity: &CanonicalEntity,
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<()> {
    log::debug!("[memory:entities] index one node_kind={node_kind}");
    index(config)?.index_entity(entity, node_id, node_kind, timestamp_ms, tree_id)
}

pub fn index_entities(
    config: &Config,
    entities: &[CanonicalEntity],
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<usize> {
    log::debug!(
        "[memory:entities] index batch count={} node_kind={node_kind}",
        entities.len()
    );
    index(config)?.index_entities(entities, node_id, node_kind, timestamp_ms, tree_id)
}

pub fn clear_entity_index_for_node(config: &Config, node_id: &str) -> Result<usize> {
    index(config)?.clear_entity_index_for_node(node_id)
}

pub fn lookup_entity(
    config: &Config,
    entity_id: &str,
    limit: Option<usize>,
) -> Result<Vec<EntityHit>> {
    index(config)?.lookup_entity(entity_id, limit)
}

pub fn list_entity_ids_for_node(config: &Config, node_id: &str) -> Result<Vec<String>> {
    index(config)?.list_entity_ids_for_node(node_id)
}

pub fn count_entity_index(config: &Config) -> Result<u64> {
    index(config)?.count_entity_index()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_entity_hit_is_the_host_facade_type() {
        let hit = EntityHit {
            entity_id: "person:alice".into(),
            node_id: "chunk-1".into(),
            node_kind: "leaf".into(),
            entity_kind: EntityKind::Person,
            surface: "Alice".into(),
            score: 1.0,
            timestamp_ms: 123,
            tree_id: Some("tree-1".into()),
            is_user: false,
        };
        assert_eq!(hit.entity_id, "person:alice");
    }
}
