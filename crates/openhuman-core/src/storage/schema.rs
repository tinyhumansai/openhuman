//! Collection declarations grouped by the crate that owns each record.
//!
//! Storage v2 has no global schema registry. At startup the host asks every
//! linked owner for its declarations and calls `ensure_collection` for each
//! one. Owners retain control of their record shapes and migrations.

use tinystoragedrivers::{CollectionSpec, IndexSpec, StorageError};

/// One linked crate's collection declarations.
#[derive(Clone, Debug)]
pub struct CollectionOwner {
    /// Cargo package that declared these collections.
    pub crate_name: &'static str,
    /// The owner's complete collection declarations.
    pub collections: Vec<CollectionSpec>,
}

/// Every linked owner's declarations, validated without combining owners.
pub fn collection_owners() -> Result<Vec<CollectionOwner>, StorageError> {
    let mut owners = declared_collections();
    for owner in &owners {
        for collection in &owner.collections {
            collection.validate()?;
        }
    }
    owners.sort_by_key(|owner| owner.crate_name);
    Ok(owners)
}

fn declared_collections() -> Vec<CollectionOwner> {
    let mut owners = vec![
        owner(
            "tinyagents-harness",
            tinyagents_harness::storage::collections(),
        ),
        owner(
            "tinyagents-session",
            tinyagents_session::port::collections(),
        ),
        owner(
            "tinyagents-tasks",
            tinyagents_tasks::storage_schema().collections(),
        ),
        owner("openhuman-core", host_collections()),
    ];

    #[cfg(feature = "skills")]
    owners.push(owner(
        "tinyskills-drivers",
        tinyskills_drivers::collections(),
    ));

    #[cfg(feature = "modules")]
    {
        owners.push(owner(
            "tinymcp-bus",
            tinymcp_bus::storage::collection_specs(),
        ));
        owners.push(owner("tinychannels-bus", channel_collection_specs()));
        owners.push(owner(
            "tinyconnectors-bus",
            tinyconnectors_bus::storage::collections()
                .into_iter()
                .map(|collection| CollectionSpec::new(collection.name))
                .collect(),
        ));
        owners.push(owner(
            "tinybox-bus",
            tinybox_bus::storage::collections()
                .into_iter()
                .map(|collection| CollectionSpec::new(collection.name))
                .collect(),
        ));
        owners.push(owner(
            "tinyjuice-bus",
            tinyjuice_bus::storage::collections(),
        ));
    }

    #[cfg(feature = "web3")]
    owners.push(owner(
        "tinywallet-bus",
        tinywallet_bus::storage::collections()
            .into_iter()
            .map(|collection| CollectionSpec::new(collection.name))
            .collect(),
    ));

    #[cfg(feature = "inbound-queues")]
    owners.push(owner("tinyeventqueues-bus", event_queue_collection_specs()));

    owners
}

fn host_collections() -> Vec<CollectionSpec> {
    let mut collections = Vec::new();
    collections.extend(crate::security::approval::collection_specs());
    collections.extend(crate::security::devices::collection_specs());
    collections.extend(crate::desktop::notifications::collection_specs());
    collections.extend(crate::integrations::task_sources::collection_specs());
    collections.extend(crate::artifacts::collection_specs());
    collections.extend(crate::platform::cost::collection_specs());
    collections.extend(crate::config::schema::config_storage_collections());
    collections.extend(crate::profiles::collection_specs());
    collections.extend(crate::threads::store::collection_specs());
    collections.extend(crate::agent::orchestration::subagent_sessions::collection_specs());
    collections.extend(super::agents::collection_specs());
    collections.extend(super::journal_maintenance::collection_specs());
    #[cfg(feature = "inbound-queues")]
    collections.extend(crate::inbound::results::collection_specs());
    collections
}

fn owner(crate_name: &'static str, collections: Vec<CollectionSpec>) -> CollectionOwner {
    CollectionOwner {
        crate_name,
        collections,
    }
}

/// Collections declared by one loadable module owner.
pub fn module_collections(owner: &str) -> Vec<CollectionSpec> {
    #[cfg(feature = "modules")]
    match owner {
        "mcp" => return module_owned_collections(tinymcp_bus::storage::collection_specs()),
        "channels" => {
            return module_owned_collections(channel_collection_specs());
        }
        "connectors" => {
            return module_owned_collections(
                tinyconnectors_bus::storage::collections()
                    .into_iter()
                    .map(|collection| CollectionSpec::new(collection.name))
                    .collect(),
            );
        }
        "box" => {
            return module_owned_collections(
                tinybox_bus::storage::collections()
                    .into_iter()
                    .map(|collection| CollectionSpec::new(collection.name))
                    .collect(),
            );
        }
        "juice" => return module_owned_collections(tinyjuice_bus::storage::collections()),
        _ => {}
    }
    #[cfg(feature = "web3")]
    if owner == "wallet" {
        return module_owned_collections(
            tinywallet_bus::storage::collections()
                .into_iter()
                .map(|collection| CollectionSpec::new(collection.name))
                .collect(),
        );
    }
    #[cfg(feature = "inbound-queues")]
    if owner == "teq" {
        return module_owned_collections(event_queue_collection_specs());
    }
    let _ = owner;
    Vec::new()
}

/// Convert TinyChannels' portable collection declarations into host storage
/// driver types without changing indexes, expiry, or search metadata.
fn channel_collection_specs() -> Vec<CollectionSpec> {
    tinychannels_bus::storage::collections()
        .into_iter()
        .map(channel_collection_spec)
        .collect()
}

fn channel_collection_spec(
    declaration: tinychannels_bus::storage::CollectionSpec,
) -> CollectionSpec {
    let mut collection = CollectionSpec::new(declaration.name);
    for index in declaration.indexes {
        let unique = index.unique;
        let index = IndexSpec::new(index.name, index.fields);
        collection = collection.index(if unique { index.unique() } else { index });
    }
    if let Some(ttl_field) = declaration.ttl_field {
        collection = collection.ttl(ttl_field);
    }
    if let Some(search) = declaration.search {
        collection = collection.searchable(search.fields);
    }
    collection
}

/// Convert the event queue bus's portable declarations into host storage types.
#[cfg(feature = "inbound-queues")]
pub(crate) fn event_queue_collection_specs() -> Vec<CollectionSpec> {
    tinyeventqueues_bus::storage::collection_specs()
        .into_iter()
        .map(|declaration| {
            declaration.indexes.into_iter().fold(
                CollectionSpec::new(declaration.name),
                |collection, index| {
                    let spec = IndexSpec::new(index.name, index.fields);
                    collection.index(if index.unique { spec.unique() } else { spec })
                },
            )
        })
        .collect()
}

/// A module may receive its own collections, never TSD's host-managed records.
fn module_owned_collections(mut collections: Vec<CollectionSpec>) -> Vec<CollectionSpec> {
    collections.retain(|collection| !collection.name.starts_with("tsd."));
    collections
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
