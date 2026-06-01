//! LLM-callable wrappers over the `people` domain (local relationship graph).
//!
//! These tools let the agent rank known contacts, resolve handles to stable
//! person ids, inspect closeness scores, attach aliases, log interactions,
//! and read a person record. Read + bounded-write tools delegate to
//! [`crate::openhuman::people::rpc`] (which returns `RpcOutcome`) or to
//! `PeopleStore` methods; results are emitted as JSON.
//!
//! All tools here are device-local and default-enabled EXCEPT
//! `people_refresh_address_book`, which performs a bulk OS address-book
//! ingest (and can trigger a Contacts permission prompt) — it is `Execute`
//! and ships default-OFF via `tools/user_filter.rs`.

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::openhuman::people::rpc;
use crate::openhuman::people::store::{self, PeopleStore};
use crate::openhuman::people::types::{Handle, Interaction, PersonId};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Acquire the global people store or surface a uniform error.
fn people_store() -> anyhow::Result<std::sync::Arc<PeopleStore>> {
    store::get().map_err(|e| anyhow::anyhow!("people store unavailable: {e}"))
}

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

fn parse_person_id(args: &serde_json::Value) -> anyhow::Result<PersonId> {
    let raw = read_required_str(args, "person_id")?;
    serde_json::from_value(json!(raw)).map_err(|e| anyhow::anyhow!("invalid person_id: {e}"))
}

/// Build a [`Handle`] from `kind` + `value` args.
fn parse_handle(args: &serde_json::Value) -> anyhow::Result<Handle> {
    let kind = read_required_str(args, "kind")?;
    let value = read_required_str(args, "value")?;
    serde_json::from_value(json!({ "kind": kind, "value": value })).map_err(|e| {
        anyhow::anyhow!("invalid handle (kind must be imessage|email|display_name): {e}")
    })
}

fn handle_schema_props() -> serde_json::Value {
    json!({
        "kind": { "type": "string", "enum": ["imessage", "email", "display_name"], "description": "Handle kind." },
        "value": { "type": "string", "description": "Handle value (phone / email / display name)." }
    })
}

/// List ranked contacts.
pub struct PeopleListTool;

#[async_trait]
impl Tool for PeopleListTool {
    fn name(&self) -> &str {
        "people_list"
    }

    fn description(&self) -> &str {
        "List the user's known contacts ranked by a closeness score (recency × \
         frequency × reciprocity × depth). Each entry carries `person_id`, \
         names, handles, the score and its components, and interaction count. \
         Use to find who the user is closest to or to resolve a name to a \
         `person_id`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1, "description": "Max contacts (default 100, cap 500)." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][people] list invoked");
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(100);
        let store = people_store()?;
        let outcome = rpc::handle_list(&store, limit)
            .await
            .map_err(|e| anyhow::anyhow!("people_list: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Resolve a handle to a person id.
pub struct PeopleResolveTool;

#[async_trait]
impl Tool for PeopleResolveTool {
    fn name(&self) -> &str {
        "people_resolve"
    }

    fn description(&self) -> &str {
        "Resolve a contact handle (kind = imessage | email | display_name) to a \
         stable `person_id`. When `create_if_missing` is true, mints a new \
         person for an unknown handle. Returns `{ person_id, created }`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "kind": handle_schema_props()["kind"],
                "value": handle_schema_props()["value"],
                "create_if_missing": { "type": "boolean", "description": "Mint a person if the handle is unknown (default false)." }
            },
            "required": ["kind", "value"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // May mint a new person record when create_if_missing is set.
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][people] resolve invoked");
        let handle = parse_handle(&args)?;
        let create = args
            .get("create_if_missing")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let store = people_store()?;
        let outcome = rpc::handle_resolve(&store, handle, create)
            .await
            .map_err(|e| anyhow::anyhow!("people_resolve: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}

/// Score breakdown for a person.
pub struct PeopleScoreTool;

#[async_trait]
impl Tool for PeopleScoreTool {
    fn name(&self) -> &str {
        "people_score"
    }

    fn description(&self) -> &str {
        "Return the closeness score and its components (recency, frequency, \
         reciprocity, depth) plus interaction count for one `person_id`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "person_id": { "type": "string", "description": "Person id (UUID)." } },
            "required": ["person_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][people] score invoked");
        let person_id = parse_person_id(&args)?;
        let store = people_store()?;
        let outcome = rpc::handle_score(&store, person_id)
            .await
            .map_err(|e| anyhow::anyhow!("people_score: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Read a full person record.
pub struct PeopleGetTool;

#[async_trait]
impl Tool for PeopleGetTool {
    fn name(&self) -> &str {
        "people_get"
    }

    fn description(&self) -> &str {
        "Load the full record for one `person_id`: display name, primary email \
         / phone, and every attached handle/alias."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "person_id": { "type": "string", "description": "Person id (UUID)." } },
            "required": ["person_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][people] get invoked");
        let person_id = parse_person_id(&args)?;
        let store = people_store()?;
        let person = store
            .get(person_id)
            .await
            .map_err(|e| anyhow::anyhow!("people_get: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "person": person,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Attach a handle alias to a person.
pub struct PeopleAddAliasTool;

#[async_trait]
impl Tool for PeopleAddAliasTool {
    fn name(&self) -> &str {
        "people_add_alias"
    }

    fn description(&self) -> &str {
        "Attach an additional handle (kind = imessage | email | display_name) \
         to an existing `person_id` so future messages from that handle map to \
         the same person. Idempotent."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "person_id": { "type": "string", "description": "Person id (UUID)." },
                "kind": handle_schema_props()["kind"],
                "value": handle_schema_props()["value"]
            },
            "required": ["person_id", "kind", "value"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][people] add_alias invoked");
        let person_id = parse_person_id(&args)?;
        let handle = parse_handle(&args)?;
        let store = people_store()?;
        store
            .add_alias(person_id, handle)
            .await
            .map_err(|e| anyhow::anyhow!("people_add_alias: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(
            &json!({ "ok": true }),
        )?))
    }
}

/// Record an interaction (append-only, feeds scoring).
pub struct PeopleRecordInteractionTool;

#[async_trait]
impl Tool for PeopleRecordInteractionTool {
    fn name(&self) -> &str {
        "people_record_interaction"
    }

    fn description(&self) -> &str {
        "Log an interaction with a `person_id` to feed the closeness score. \
         `is_outbound` marks who initiated; `length` is a depth proxy (e.g. \
         message length). Timestamp defaults to now. Append-only."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "person_id": { "type": "string", "description": "Person id (UUID)." },
                "is_outbound": { "type": "boolean", "description": "True if the user sent it (required)." },
                "length": { "type": "integer", "minimum": 0, "description": "Depth proxy (default 0)." }
            },
            "required": ["person_id", "is_outbound"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][people] record_interaction invoked");
        let person_id = parse_person_id(&args)?;
        let is_outbound = args
            .get("is_outbound")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| anyhow::anyhow!("missing required boolean argument `is_outbound`"))?;
        let length = args
            .get("length")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let interaction = Interaction {
            person_id,
            ts: Utc::now(),
            is_outbound,
            length,
        };
        let store = people_store()?;
        store
            .record_interaction(interaction)
            .await
            .map_err(|e| anyhow::anyhow!("people_record_interaction: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(
            &json!({ "ok": true }),
        )?))
    }
}

/// Bulk-ingest the OS address book. **Triggers a permission prompt** —
/// default-OFF.
pub struct PeopleRefreshAddressBookTool;

#[async_trait]
impl Tool for PeopleRefreshAddressBookTool {
    fn name(&self) -> &str {
        "people_refresh_address_book"
    }

    fn description(&self) -> &str {
        "Bulk-import the operating system address book into the people store, \
         seeding contacts and their handles. On macOS this may trigger a \
         Contacts (TCC) permission prompt. Returns counts of seeded / skipped \
         contacts. Only use when the user explicitly asks to import contacts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][people] refresh_address_book invoked");
        let store = people_store()?;
        let outcome = rpc::handle_refresh_address_book(&store)
            .await
            .map_err(|e| anyhow::anyhow!("people_refresh_address_book: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    #[test]
    fn names_and_levels() {
        assert_eq!(PeopleListTool.name(), "people_list");
        assert_eq!(PeopleListTool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(PeopleResolveTool.permission_level(), PermissionLevel::Write);
        assert_eq!(
            PeopleRecordInteractionTool.permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(
            PeopleRefreshAddressBookTool.permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(PeopleListTool.scope(), ToolScope::All);
    }

    #[test]
    fn parse_handle_accepts_known_kinds() {
        let h = parse_handle(&json!({ "kind": "email", "value": "a@b.com" })).expect("email");
        assert!(matches!(h, Handle::Email(_)));
        let d =
            parse_handle(&json!({ "kind": "display_name", "value": "Alice" })).expect("display");
        assert!(matches!(d, Handle::DisplayName(_)));
    }

    #[test]
    fn parse_handle_rejects_unknown_kind() {
        let err = parse_handle(&json!({ "kind": "fax", "value": "x" })).expect_err("bad kind");
        assert!(err.to_string().contains("handle"));
    }

    #[test]
    fn parse_person_id_rejects_non_uuid() {
        let err = parse_person_id(&json!({ "person_id": "not-a-uuid" })).expect_err("bad uuid");
        assert!(err.to_string().contains("person_id"));
    }

    #[tokio::test]
    async fn score_requires_person_id() {
        let err = PeopleScoreTool
            .execute(json!({}))
            .await
            .expect_err("missing person_id");
        assert!(err.to_string().contains("person_id"));
    }
}
