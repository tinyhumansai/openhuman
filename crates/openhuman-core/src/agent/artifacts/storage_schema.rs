//! The `artifacts` owner's storage schema: the artifact cards, the arguments
//! that regenerate them, and the versioned records, with the steps that bring
//! the v1 collections forward.
//!
//! | v1 | v2 | change |
//! |---|---|---|
//! | `artifacts` (`created_at` RFC 3339 beside `created_ms`) | `artifacts.records` | `created_at` dropped |
//! | `artifact_args` (`{args}`) | `artifacts.args` | `args` unwrapped: the body is the argument object |
//! | `artifact_versions` (camelCase `ArtifactRecord`) | `artifacts.versions` | snake_case keys, `*AtMillis` becomes `*_ms` |
//!
//! Arguments that are not a JSON object (no producer writes any) are kept as
//! `{"_args": <value>}`, so the body is always an object and
//! [`args_of_body`] reads either form back.

use serde_json::{json, Value};
use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, IndexSpec, Result};

use super::types::ArtifactMeta;
use crate::storage::schema_support::{camel_keys, key_of, owner, snake_keys, time_ms};

pub(super) const RECORDS: &str = "artifacts.records";
pub(super) const ARGS: &str = "artifacts.args";
pub(super) const VERSIONS: &str = "artifacts.versions";
pub(super) const LEGACY_RECORDS: &str = "artifacts";
pub(super) const LEGACY_ARGS: &str = "artifact_args";
pub(super) const LEGACY_VERSIONS: &str = "artifact_versions";
const SCHEMA: u32 = 2;

/// The v2 names of the `*AtMillis` fields of `tiny_artifacts::ArtifactRecord`.
const RECORD_TIME_KEYS: [(&str, &str); 4] = [
    ("createdAtMillis", "created_ms"),
    ("updatedAtMillis", "updated_ms"),
    ("expiresAtMillis", "expires_ms"),
    ("publishedAtMillis", "published_ms"),
];

/// What the artifact stores keep and the steps from the v1 collections.
#[must_use]
pub fn storage_schema() -> Schema {
    Schema::new(owner("artifacts"))
        .collection_with(SCHEMA, records_spec(), ScopeClass::Tenant)
        .collection_with(SCHEMA, CollectionSpec::new(ARGS), ScopeClass::Tenant)
        .collection_with(SCHEMA, CollectionSpec::new(VERSIONS), ScopeClass::Tenant)
        .step(Step::new("records_v2", RECORDS, 1, SCHEMA, records_v2).source(LEGACY_RECORDS))
        .step(Step::new("args_v2", ARGS, 1, SCHEMA, args_v2).source(LEGACY_ARGS))
        .step(Step::new("versions_v2", VERSIONS, 1, SCHEMA, versions_v2).source(LEGACY_VERSIONS))
}

pub(super) fn records_spec() -> CollectionSpec {
    CollectionSpec::new(RECORDS).index(IndexSpec::new("by_thread", ["thread_id", "created_ms"]))
}

/// A card as its stored body: its fields with `created_at` replaced by
/// `created_ms`.
///
/// # Errors
///
/// When the card does not serialize.
pub(super) fn meta_to_body(meta: &ArtifactMeta) -> anyhow::Result<Value> {
    let mut doc = serde_json::to_value(meta)?;
    if let Some(fields) = doc.as_object_mut() {
        fields.remove("created_at");
        fields.insert(
            "created_ms".into(),
            meta.created_at.timestamp_millis().into(),
        );
    }
    Ok(doc)
}

/// The card a stored body holds.
///
/// # Errors
///
/// When the body is not a card.
pub(super) fn meta_from_body(mut doc: Value) -> anyhow::Result<ArtifactMeta> {
    if let Some(fields) = doc.as_object_mut() {
        let created_ms = fields
            .remove("created_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let at = chrono::DateTime::from_timestamp_millis(created_ms).unwrap_or_default();
        fields.insert("created_at".into(), json!(at));
    }
    Ok(serde_json::from_value(doc)?)
}

/// The stored body of arguments `args`.
pub(super) fn args_body(args: &Value) -> Value {
    if args.is_object() {
        args.clone()
    } else {
        json!({ "_args": args })
    }
}

/// The arguments a stored body holds.
pub(super) fn args_of_body(body: &Value) -> Value {
    match body.as_object() {
        Some(fields) if fields.len() == 1 && fields.contains_key("_args") => body["_args"].clone(),
        _ => body.clone(),
    }
}

/// The stored body of a versioned record, and back.
pub(super) fn version_body(record: &Value) -> Value {
    snake_keys(record, &RECORD_TIME_KEYS)
}

pub(super) fn version_record(body: &Value) -> Value {
    camel_keys(body, &RECORD_TIME_KEYS)
}

fn records_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let mut body = record.body;
            let created_ms = body
                .get("created_ms")
                .and_then(Value::as_i64)
                .or_else(|| time_ms(&body, "created_at"))
                .unwrap_or(record.meta.created_ms);
            if let Some(fields) = body.as_object_mut() {
                fields.remove("created_at");
                fields.insert("created_ms".into(), created_ms.into());
            }
            Ok(TargetRecord::new(RECORDS, key_of(&record.id)?, body).created_ms(created_ms))
        })
        .collect()
}

fn args_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let args = record.body.get("args").cloned().unwrap_or(Value::Null);
            Ok(
                TargetRecord::new(ARGS, key_of(&record.id)?, args_body(&args))
                    .created_ms(record.meta.created_ms),
            )
        })
        .collect()
}

fn versions_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            // v1 added `updated_ms` beside the record's own `updatedAtMillis`;
            // the renamed field replaces it.
            let mut body = record.body;
            if let Some(fields) = body.as_object_mut() {
                fields.remove("updated_ms");
            }
            Ok(
                TargetRecord::new(VERSIONS, key_of(&record.id)?, version_body(&body))
                    .created_ms(record.meta.created_ms),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
