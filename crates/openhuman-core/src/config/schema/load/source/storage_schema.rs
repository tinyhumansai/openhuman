//! The `config` owner's storage schema: the per-scope config document and the
//! step that brings the v1 collection forward.
//!
//! | v1 | v2 | change |
//! |---|---|---|
//! | `config` (id = the scope string) | `config.documents` (id `Key::of([scope])`) | rename; `toml_enc` (an encrypted value) is kept as is |

use tinystoragedrivers::CollectionSpec;

use crate::storage::schema_support::{key_of, map_legacy_collection, LegacyTargetRecord};

/// The v2 collection holding one config document per scope.
pub(crate) const CONFIG_COLLECTION: &str = "config.documents";
/// The v1 name.
pub(crate) const LEGACY_CONFIG_COLLECTION: &str = "config";
/// The config source's persisted collection declaration.
pub(crate) fn collection_specs() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(CONFIG_COLLECTION)]
}

/// Converts the previous per-scope config document into its owner collection.
pub(crate) fn upgrade_records(
    records: Vec<crate::storage::import::Record>,
) -> anyhow::Result<Vec<crate::storage::import::Record>> {
    map_legacy_collection(records, LEGACY_CONFIG_COLLECTION, |batch| {
        batch
            .into_iter()
            .map(|record| {
                Ok(
                    LegacyTargetRecord::new(CONFIG_COLLECTION, key_of(&record.id)?, record.body)
                        .created_ms(record.meta.created_ms),
                )
            })
            .collect()
    })
}

/// The document id of the config of `scope`.
///
/// # Errors
///
/// When `scope` is empty.
pub(crate) fn document_id(
    scope: &str,
) -> std::result::Result<String, tinystoragedrivers::StorageError> {
    key_of(scope)
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
