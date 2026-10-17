//! The `cost` owner's storage schema: the usage ledger and the step that
//! brings the v1 collection forward.
//!
//! | v1 | v2 | change |
//! |---|---|---|
//! | `cost_records` (`usage.timestamp` RFC 3339 beside `ts_ms`) | `cost.records` | `usage.timestamp` dropped; `ts_ms` is the instant |

use serde_json::Value;
use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, IndexSpec, Result};

use super::types::CostRecord;
use crate::storage::schema_support::{key_of, owner, rfc3339_ms};

pub(super) const RECORDS: &str = "cost.records";
pub(super) const LEGACY_RECORDS: &str = "cost_records";
const SCHEMA: u32 = 2;

/// What the cost ledger keeps and the step from the v1 collection.
#[must_use]
pub fn storage_schema() -> Schema {
    Schema::new(owner("cost"))
        .collection_with(SCHEMA, spec(), ScopeClass::Tenant)
        .step(Step::new("records_v2", RECORDS, 1, SCHEMA, records_v2).source(LEGACY_RECORDS))
}

pub(super) fn spec() -> CollectionSpec {
    CollectionSpec::new(RECORDS).index(IndexSpec::new("by_time", ["ts_ms"]))
}

/// A record as its stored body: its fields with the usage timestamp carried
/// once, as `ts_ms`.
///
/// # Errors
///
/// When the record does not serialize.
pub(super) fn to_body(record: &CostRecord) -> anyhow::Result<Value> {
    let mut doc = serde_json::to_value(record)?;
    if let Some(usage) = doc.get_mut("usage").and_then(Value::as_object_mut) {
        usage.remove("timestamp");
    }
    doc["ts_ms"] = record.usage.timestamp.timestamp_millis().into();
    Ok(doc)
}

/// The record a stored body holds.
///
/// # Errors
///
/// When the body is not a record.
pub(super) fn from_body(mut doc: Value) -> anyhow::Result<CostRecord> {
    let ts_ms = doc.get("ts_ms").and_then(Value::as_i64).unwrap_or_default();
    let at = chrono::DateTime::from_timestamp_millis(ts_ms).unwrap_or_default();
    if let Some(usage) = doc.get_mut("usage").and_then(Value::as_object_mut) {
        usage.insert("timestamp".into(), serde_json::json!(at));
    }
    Ok(serde_json::from_value(doc)?)
}

fn records_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let mut body = record.body;
            let stamped = body
                .pointer("/usage/timestamp")
                .and_then(Value::as_str)
                .and_then(rfc3339_ms);
            let ts_ms = body
                .get("ts_ms")
                .and_then(Value::as_i64)
                .or(stamped)
                .unwrap_or(record.meta.created_ms);
            if let Some(usage) = body.get_mut("usage").and_then(Value::as_object_mut) {
                usage.remove("timestamp");
            }
            body["ts_ms"] = ts_ms.into();
            Ok(TargetRecord::new(RECORDS, key_of(&record.id)?, body).created_ms(ts_ms))
        })
        .collect()
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
