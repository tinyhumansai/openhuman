//! The `subagents` owner's storage schema: the durable sub-agent session
//! records and the step that brings the v1 collection forward.
//!
//! | v1 | v2 | scope |
//! |---|---|---|
//! | `subagent_sessions` (camelCase body, RFC 3339 `createdAt`/`updatedAt`/`lastUsedAt`, snake_case `created_at` copy) | `subagents.sessions` (snake_case body, `created_ms`/`updated_ms`/`last_used_ms`) | agent |
//!
//! `updated_ms` stays a body field rather than only tsd metadata: it is the
//! writer's own clock, which `store_documents::write_if_newer` compares so a
//! caller holding an older snapshot never overwrites a session another core
//! saved since. The driver's `updated_ms` is the time of the write, not of the
//! session's last use.

use serde_json::{Map, Value};
use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, Result};

use crate::storage::schema_support::{key_of, owner, time_ms};

pub(super) const SESSIONS: &str = "subagents.sessions";
pub(super) const LEGACY_SESSIONS: &str = "subagent_sessions";
const SCHEMA: u32 = 2;

/// What the session store keeps and the step from the v1 collection.
#[must_use]
pub fn storage_schema() -> Schema {
    Schema::new(owner("subagents"))
        .collection_with(SCHEMA, spec(), ScopeClass::Agent)
        .step(Step::new("sessions_v2", SESSIONS, 1, SCHEMA, sessions_v2).source(LEGACY_SESSIONS))
}

pub(super) fn spec() -> CollectionSpec {
    CollectionSpec::new(SESSIONS)
}

pub(super) fn legacy_collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(LEGACY_SESSIONS)]
}

/// The camelCase v1 field and its snake_case v2 name, for the plain copies.
const COPIES: [(&str, &str); 16] = [
    ("subagentSessionId", "subagent_session_id"),
    ("parentSession", "parent_session"),
    ("parentThreadId", "parent_thread_id"),
    ("workerThreadId", "worker_thread_id"),
    ("agentId", "agent_id"),
    ("displayName", "display_name"),
    ("model", "model"),
    ("sandboxMode", "sandbox_mode"),
    ("actionRoot", "action_root"),
    ("taskKey", "task_key"),
    ("taskTitle", "task_title"),
    ("currentTaskId", "current_task_id"),
    ("status", "status"),
    ("reusable", "reusable"),
    ("latestHistory", "latest_history"),
    ("latestError", "latest_error"),
];

fn sessions_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let old = &record.body;
            let mut body = Map::new();
            for (from, to) in COPIES {
                if let Some(value) = old.get(from).filter(|value| !value.is_null()) {
                    body.insert(to.to_owned(), value.clone());
                }
            }
            let created = time_ms(old, "createdAt")
                .or_else(|| time_ms(old, "created_at"))
                .unwrap_or(record.meta.created_ms);
            let updated = time_ms(old, "updatedAt").unwrap_or(created);
            let last_used = time_ms(old, "lastUsedAt").unwrap_or(updated);
            body.insert("created_ms".into(), created.into());
            body.insert("updated_ms".into(), updated.into());
            body.insert("last_used_ms".into(), last_used.into());
            let id = old
                .get("subagentSessionId")
                .and_then(Value::as_str)
                .unwrap_or(&record.id);
            Ok(TargetRecord::new(SESSIONS, key_of(id)?, Value::Object(body)).created_ms(created))
        })
        .collect()
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
