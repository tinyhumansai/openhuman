//! The `approvals` owner's storage schema: the request log, the per-flow
//! trust grants, and the steps that bring the v1 collections forward.
//!
//! | v1 | v2 | scope |
//! |---|---|---|
//! | `approvals` (RFC 3339 `created_at`/`expires_at`/`decided_at`/`executed_at`, `expires_ts`) | `approvals.requests` (`created_ms`, `deadline_ms`, `decided_ms`, `executed_ms`) | tenant |
//! | `approval_flow_trust` (id `<len>:<flow>/<tool>`) | `approvals.flow_trust` (id `Key::of([flow_id, tool_name])`) | tenant |
//!
//! `deadline_ms` is the instant an undecided request is denied. It is a body
//! field, not tsd's `expires_ms`, because a lapsed request stays in the audit
//! trail instead of reading as absent. `args_redacted` and `source_context`
//! stay JSON text: they carry arbitrary tool arguments whose keys a
//! document-store driver (MongoDB) must never see as field names.

use anyhow::Result;
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, IndexSpec};

use crate::storage::schema_support::{
    key_of, key_of_parts, map_legacy_collection, time_ms, Body, LegacySourceRecord as SourceRecord,
    LegacyTargetRecord as TargetRecord,
};

/// The v2 request collection: one document per request id.
pub(super) const APPROVALS: &str = "approvals.requests";
/// The v2 grant collection: one document per `(flow_id, tool_name)`.
pub(super) const FLOW_TRUST: &str = "approvals.flow_trust";
/// The v1 request collection, still written by the legacy `approval.db` importer.
pub(super) const LEGACY_APPROVALS: &str = "approvals";
/// The v1 grant collection.
pub(super) const LEGACY_FLOW_TRUST: &str = "approval_flow_trust";
/// The body version of both collections.
/// Converts approval legacy documents into this owner's current record shapes.
pub(crate) fn upgrade_records(
    records: Vec<crate::storage::import::Record>,
) -> Result<Vec<crate::storage::import::Record>> {
    let records = map_legacy_collection(records, LEGACY_APPROVALS, requests_v2)?;
    map_legacy_collection(records, LEGACY_FLOW_TRUST, flow_trust_v2)
}

/// The two v2 collections with their indexes.
pub(super) fn specs() -> [CollectionSpec; 2] {
    [
        CollectionSpec::new(APPROVALS)
            .index(IndexSpec::new("by_pending", ["pending", "created_ms"]))
            .index(IndexSpec::new("by_decided", ["pending", "decided_ms"]))
            .index(IndexSpec::new("by_session", ["session_id", "pending"])),
        CollectionSpec::new(FLOW_TRUST).index(IndexSpec::new("by_flow", ["flow_id", "tool_name"])),
    ]
}

/// Collections owned by the approval store.
pub(crate) fn collection_specs() -> Vec<CollectionSpec> {
    specs().into()
}

/// The v1 collections as the legacy importer declares them.
pub(super) fn legacy_collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(LEGACY_APPROVALS)
            .index(IndexSpec::new("by_pending", ["pending", "created_at"]))
            .index(IndexSpec::new("by_decided", ["pending", "decided_at"]))
            .index(IndexSpec::new("by_session", ["session_id", "pending"])),
        CollectionSpec::new(LEGACY_FLOW_TRUST)
            .index(IndexSpec::new("by_flow", ["flow_id", "tool_name"])),
    ]
}

/// The document id of a `(flow_id, tool_name)` grant.
///
/// # Panics
///
/// Never in practice: a grant has two parts, and only a lone empty part is
/// refused.
pub(super) fn trust_id(flow_id: &str, tool_name: &str) -> String {
    #[allow(clippy::expect_used)]
    key_of_parts([flow_id, tool_name]).expect("a two-part key is valid")
}

fn requests_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let old = &record.body;
            let created_ms = time_ms(old, "created_at").unwrap_or(record.meta.created_ms);
            let body = Body::new()
                .copy(old, "tool_name")
                .copy(old, "action_summary")
                .copy(old, "args_redacted")
                .copy(old, "session_id")
                .copy(old, "source_context")
                .copy(old, "tool_call_id")
                .copy(old, "agent_id")
                .copy(old, "decision")
                .copy(old, "execution_outcome")
                .copy(old, "execution_error")
                .set("created_ms", created_ms)
                .set_some("deadline_ms", time_ms(old, "expires_at"))
                .set_some("decided_ms", time_ms(old, "decided_at"))
                .set_some("executed_ms", time_ms(old, "executed_at"))
                .set(
                    "pending",
                    old.get("pending").and_then(Value::as_bool).unwrap_or(false),
                )
                .build();
            Ok(TargetRecord::new(APPROVALS, key_of(&record.id)?, body)
                .created_ms(record.meta.created_ms)
                .source_metadata(record.meta))
        })
        .collect()
}

fn flow_trust_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    Ok(batch
        .into_iter()
        .filter_map(|record| {
            let old = &record.body;
            // A grant is identified by its two parts; a row missing either is
            // not a grant this build could ever have written.
            let flow_id = old.get("flow_id").and_then(Value::as_str)?;
            let tool_name = old.get("tool_name").and_then(Value::as_str)?;
            let created_ms = time_ms(old, "created_at").unwrap_or(record.meta.created_ms);
            let body = Body::new()
                .set("flow_id", flow_id)
                .set("tool_name", tool_name)
                .set("created_ms", created_ms)
                .build();
            Some(
                TargetRecord::new(FLOW_TRUST, trust_id(flow_id, tool_name), body)
                    .created_ms(record.meta.created_ms)
                    .source_metadata(record.meta),
            )
        })
        .collect())
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
