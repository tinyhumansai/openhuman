//! The composed storage schema: every owner's `storage_schema()` in one
//! [`Registry`].
//!
//! Each crate that stores records declares what it stores (collections,
//! streams, blob prefixes, and the v1 -> v2 steps that bring older data
//! forward). The host composes them once, `prepare`s the backend from the
//! result, migrates every scope ([`super::boot`]) and generates each module's
//! storage grant from the module-scoped items
//! ([`crate::modules::storage_grants`]). Composition refuses a name outside its
//! owner's prefix, a name claimed twice, a version that goes down and a step
//! gap, so a collision between two owners fails here, at boot and in
//! `scripts/ci/check-storage-schema.mjs`, never at a first write.
//!
//! Module owners come from their `*-bus` contract crates (optional `storage` feature),
//! never from the implementation libraries. Owners whose crate is gated off in a build are absent from its registry, as
//! their data is.

use std::sync::OnceLock;

use tinystoragedrivers::schema::{Item, Registry, Schema, ScopeClass};

/// Every owner's schema that this build links.
pub fn owner_schemas() -> Vec<Schema> {
    let mut schemas = vec![
        tinyagents_harness::storage_schema(),
        tinyagents_session::storage_schema(),
        tinyagents_session::ledger_storage_schema(),
        tinyagents_session::history_storage_schema(),
        tinyagents_session::conversations_storage_schema(),
        tinyagents_graph::storage_schema(),
        tinyagents_tasks::storage_schema(),
        crate::security::approval::storage_schema(),
        crate::security::devices::storage_schema(),
        crate::desktop::notifications::storage_schema(),
        crate::integrations::task_sources::storage_schema(),
        crate::agent::artifacts::storage_schema(),
        crate::platform::cost::storage_schema(),
        crate::config::schema::config_storage_schema(),
        crate::profiles::storage_schema(),
        crate::threads::store::storage_schema(),
        crate::agent::orchestration::subagent_sessions::storage_schema(),
        super::agents::storage_schema(),
    ];
    #[cfg(feature = "flows")]
    {
        schemas.push(tinyflows_drivers::storage_schema());
        schemas.push(tinyflows_drivers::cron_storage_schema());
    }
    #[cfg(feature = "skills")]
    schemas.push(tinyskills_drivers::storage_schema());
    #[cfg(feature = "modules")]
    {
        schemas.push(tinymcp_bus::storage::storage_schema());
        schemas.push(tinychannels_bus::storage::storage_schema());
        schemas.push(tinyconnectors_bus::storage::storage_schema());
        schemas.push(tinybox_bus::storage::storage_schema());
        schemas.push(tinyjuice_bus::storage::storage_schema());
    }
    #[cfg(feature = "web3")]
    schemas.push(tinywallet_bus::storage::storage_schema());
    #[cfg(feature = "tinymemes")]
    schemas.push(tinymemes_bus::storage::storage_schema());
    schemas
}

/// The registry composed from [`owner_schemas`].
///
/// # Errors
///
/// The composition error naming the owner and item at fault. A build that
/// links two owners claiming one name never gets this far in CI.
pub fn registry() -> Result<&'static Registry, super::StorageError> {
    static REGISTRY: OnceLock<Result<Registry, String>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Registry::compose(owner_schemas()).map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|message| super::StorageError::invalid_input(message.clone()))
}

/// The module-scoped declarations of owner `owner`, for the module's grant:
/// `(collections, stream names, blob prefixes, secret prefixes)`.
pub fn module_items(owner: &str) -> ModuleItems {
    let mut items = ModuleItems::default();
    for schema in owner_schemas()
        .into_iter()
        .filter(|schema| schema.owner.as_str() == owner)
    {
        for item in schema.items {
            if item.scope() != ScopeClass::Module {
                continue;
            }
            match item {
                Item::Collection { spec, .. } => items.collections.push(spec),
                Item::Stream { name, .. } => items.streams.push(name),
                Item::Blobs { prefix, .. } => items.blobs.push(prefix),
                Item::Secrets { prefix, .. } => items.secrets.push(prefix),
            }
        }
    }
    items
}

/// What one owner declares at module scope.
#[derive(Debug, Default, Clone)]
pub struct ModuleItems {
    /// Collections, with their indexes.
    pub collections: Vec<tinystoragedrivers::CollectionSpec>,
    /// Stream family names.
    pub streams: Vec<String>,
    /// Blob key prefixes.
    pub blobs: Vec<String>,
    /// Secret key prefixes.
    pub secrets: Vec<String>,
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
