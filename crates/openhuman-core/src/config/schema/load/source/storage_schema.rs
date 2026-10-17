//! The `config` owner's storage schema: the per-scope config document and the
//! step that brings the v1 collection forward.
//!
//! | v1 | v2 | change |
//! |---|---|---|
//! | `config` (id = the scope string) | `config.documents` (id `Key::of([scope])`) | rename; `toml_enc` (an encrypted value) is kept as is |

use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, Result};

use crate::storage::schema_support::{key_of, owner};

/// The v2 collection holding one config document per scope.
pub(crate) const CONFIG_COLLECTION: &str = "config.documents";
/// The v1 name.
pub(crate) const LEGACY_CONFIG_COLLECTION: &str = "config";
const SCHEMA: u32 = 2;

/// What the config source keeps and the step from the v1 collection.
#[must_use]
pub fn storage_schema() -> Schema {
    Schema::new(owner("config"))
        .collection_with(
            SCHEMA,
            CollectionSpec::new(CONFIG_COLLECTION),
            ScopeClass::Tenant,
        )
        .step(
            Step::new("documents_v2", CONFIG_COLLECTION, 1, SCHEMA, documents_v2)
                .source(LEGACY_CONFIG_COLLECTION),
        )
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

fn documents_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            Ok(
                TargetRecord::new(CONFIG_COLLECTION, document_id(&record.id)?, record.body)
                    .created_ms(record.meta.created_ms),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
