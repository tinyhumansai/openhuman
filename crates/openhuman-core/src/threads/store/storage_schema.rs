//! The `threads` owner's storage schema: what the host keeps about a
//! conversation beside the conversation records TinyAgents owns
//! (`conversations.*`).
//!
//! | v1 | v2 | scope |
//! |---|---|---|
//! | `threads_transcript_keys` (id = thread id) | `threads.transcript_keys` (id `Key::of([thread_id])`) | tenant |
//!
//! The importer marker `threads_import` is not a collection any more: the
//! legacy JSONL import is a `tsd.migrations` entry
//! ([`crate::storage::import_state`]).

use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, Result};

use crate::storage::schema_support::{key_of, owner};

pub(super) const TRANSCRIPT_KEYS: &str = "threads.transcript_keys";
pub(super) const LEGACY_TRANSCRIPT_KEYS: &str = "threads_transcript_keys";
const SCHEMA: u32 = 2;

/// What the thread host keeps and the step from the v1 collection.
#[must_use]
pub fn storage_schema() -> Schema {
    Schema::new(owner("threads"))
        .collection_with(SCHEMA, spec(), ScopeClass::Tenant)
        .step(
            Step::new(
                "transcript_keys_v2",
                TRANSCRIPT_KEYS,
                1,
                SCHEMA,
                transcript_keys_v2,
            )
            .source(LEGACY_TRANSCRIPT_KEYS),
        )
}

pub(super) fn spec() -> CollectionSpec {
    CollectionSpec::new(TRANSCRIPT_KEYS)
}

fn transcript_keys_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            Ok(
                TargetRecord::new(TRANSCRIPT_KEYS, key_of(&record.id)?, record.body)
                    .created_ms(record.meta.created_ms),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
