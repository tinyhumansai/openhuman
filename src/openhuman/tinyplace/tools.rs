//! LLM-callable agent tools for the `tinyplace` domain.
//!
//! Exposes write actions (job proposal submission) to the agent tool-call
//! pipeline. The candidate is always resolved server-side from the wallet
//! signer (`signer.agent_id()`) to prevent impersonation — it is never
//! accepted as a tool argument.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tinyplace::types::ProposalCreateRequest;

use crate::core::all::{ControllerHandler, RegisteredController};
use crate::core::{FieldSchema, TypeSchema};
use crate::openhuman::tinyplace::ops::{global_state, map_err};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

const LOG_PREFIX: &str = "[tinyplace][tool]";

// ── Tinyplace controller-backed tools ────────────────────────────────────────

/// Agent-callable wrapper around an existing tiny.place controller.
///
/// These tools intentionally reuse the internal controller handlers rather than
/// duplicating request construction. That keeps validation, client lookup, x402
/// confirmation, and tiny.place SDK behaviour identical between RPC and agent
/// tool calls.
#[derive(Clone)]
pub struct TinyplaceControllerTool {
    schema: crate::core::ControllerSchema,
    handler: ControllerHandler,
    tool_name: String,
    permission_level: PermissionLevel,
    external_effect: bool,
    concurrency_safe: bool,
}

impl TinyplaceControllerTool {
    fn from_controller(controller: RegisteredController) -> Self {
        let function = controller.schema.function;
        let write = is_write_function(function);
        let tool_name = format!("tinyplace_{function}");

        Self {
            schema: controller.schema,
            handler: controller.handler,
            tool_name,
            permission_level: if write {
                PermissionLevel::Write
            } else {
                PermissionLevel::ReadOnly
            },
            external_effect: write && has_external_effect(function),
            concurrency_safe: !write,
        }
    }
}

#[async_trait]
impl Tool for TinyplaceControllerTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        self.schema.description
    }

    fn parameters_schema(&self) -> Value {
        controller_parameters_schema(&self.schema.inputs)
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let params = match args {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            other => {
                return Ok(ToolResult::error(format!(
                    "{} expects a JSON object argument, got {}",
                    self.name(),
                    value_kind(&other)
                )));
            }
        };

        let param_keys: Vec<&str> = params.keys().map(String::as_str).collect();
        log::debug!(
            "{LOG_PREFIX} {} start param_keys={:?}",
            self.name(),
            param_keys
        );

        match (self.handler)(params).await {
            Ok(value) => {
                log::debug!("{LOG_PREFIX} {} success", self.name());
                Ok(ToolResult::json(value))
            }
            Err(message) => {
                log::warn!("{LOG_PREFIX} {} failed: {message}", self.name());
                Ok(ToolResult::error(message))
            }
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission_level
    }

    fn external_effect(&self) -> bool {
        self.external_effect
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        self.concurrency_safe
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        Some(64 * 1024)
    }
}

/// All tiny.place controller tools available to the dedicated tinyplace agent.
pub fn all_tinyplace_agent_tools() -> Vec<Box<dyn Tool>> {
    crate::openhuman::tinyplace::all_tinyplace_registered_controllers()
        .into_iter()
        .map(|controller| {
            Box::new(TinyplaceControllerTool::from_controller(controller)) as Box<dyn Tool>
        })
        .collect()
}

fn controller_parameters_schema(inputs: &[FieldSchema]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for input in inputs {
        properties.insert(
            input.name.to_string(),
            type_schema_to_json_schema(&input.ty, input.comment),
        );
        if input.required {
            required.push(Value::String(input.name.to_string()));
        }
    }

    let mut schema = serde_json::Map::from_iter([
        ("type".to_string(), Value::String("object".to_string())),
        ("additionalProperties".to_string(), Value::Bool(false)),
        ("properties".to_string(), Value::Object(properties)),
    ]);

    if !required.is_empty() {
        schema.insert("required".to_string(), Value::Array(required));
    }

    Value::Object(schema)
}

fn type_schema_to_json_schema(ty: &TypeSchema, description: &'static str) -> Value {
    let mut schema = match ty {
        TypeSchema::Bool => json!({ "type": "boolean" }),
        TypeSchema::I64 | TypeSchema::U64 => json!({ "type": "integer" }),
        TypeSchema::F64 => json!({ "type": "number" }),
        TypeSchema::String => json!({ "type": "string" }),
        TypeSchema::Json => json!({}),
        TypeSchema::Bytes => json!({ "type": "string", "contentEncoding": "base64" }),
        TypeSchema::Array(item) => {
            json!({ "type": "array", "items": type_schema_to_json_schema(item, "") })
        }
        TypeSchema::Map(value) => {
            json!({
                "type": "object",
                "additionalProperties": type_schema_to_json_schema(value, "")
            })
        }
        TypeSchema::Option(inner) => {
            let mut inner_schema = type_schema_to_json_schema(inner, description);
            if let Value::Object(map) = &mut inner_schema {
                let nullable_type = match map.remove("type") {
                    Some(Value::String(name)) => {
                        Value::Array(vec![Value::String(name), Value::String("null".to_string())])
                    }
                    Some(Value::Array(mut names)) => {
                        if !names.iter().any(|name| name.as_str() == Some("null")) {
                            names.push(Value::String("null".to_string()));
                        }
                        Value::Array(names)
                    }
                    Some(other) => other,
                    None => return inner_schema,
                };
                map.insert("type".to_string(), nullable_type);
            }
            return inner_schema;
        }
        TypeSchema::Enum { variants } => {
            json!({ "type": "string", "enum": variants })
        }
        TypeSchema::Object { fields } => controller_parameters_schema(fields),
        TypeSchema::Ref(name) => json!({
            "type": "object",
            "description": format!("{description} Shape: {name}."),
            "additionalProperties": true
        }),
    };

    if !description.is_empty() {
        if let Value::Object(map) = &mut schema {
            map.entry("description".to_string())
                .or_insert_with(|| Value::String(description.to_string()));
        }
    }

    schema
}

fn is_write_function(function: &str) -> bool {
    const WRITE_FUNCTIONS: &[&str] = &[
        "bounties_approve",
        "bounties_cancel",
        "bounties_comment",
        "bounties_create",
        "bounties_run_council",
        "bounties_submit",
        "broadcasts_subscribe",
        "broadcasts_unsubscribe",
        "channels_join",
        "channels_leave",
        "feedback_create",
        "feedback_vote",
        "feeds_add_comment",
        "feeds_create_post",
        "feeds_delete_comment",
        "feeds_delete_post",
        "feeds_like_post",
        "feeds_unlike_post",
        "follows_follow",
        "follows_unfollow",
        "groups_create_invite",
        "groups_join",
        "groups_leave",
        "groups_redeem_invite",
        "groups_revoke_invite",
        "groups_set_member_role",
        "inbox_archive",
        "inbox_mark_all_read",
        "inbox_mark_read",
        "inbox_remove",
        "inbox_unarchive",
        "jobs_adjudicate_dispute",
        "jobs_apply",
        "jobs_cancel",
        "jobs_create",
        "jobs_open_dispute",
        "jobs_select",
        "jobs_shortlist_proposal",
        "jobs_withdraw_proposal",
        "marketplace_bid",
        "marketplace_buy_identity",
        "marketplace_buy_product",
        "marketplace_offer",
        "messages_acknowledge",
        "registry_register",
        "signal_provision",
        "signal_decrypt_message",
        "signal_register_encryption_key",
        "signal_rotate_signed_pre_key",
        "signal_send_message",
        "signal_upload_pre_keys",
        "solana_call",
        "streams_start",
        "streams_stop",
        "users_confirm_email_verification",
        "users_start_email_verification",
        "users_update_profile",
    ];

    WRITE_FUNCTIONS.contains(&function)
}

fn has_external_effect(_function: &str) -> bool {
    true
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ── TinyplaceJobApplyTool ─────────────────────────────────────────────────────

/// Submit a proposal (apply) to an open tiny.place job on behalf of the user.
///
/// The candidate identity is always derived from the user's wallet signer
/// (`signer.agent_id()`) — it cannot be overridden via tool arguments,
/// preventing any impersonation of another user.
///
/// Job proposals are free (directory-signed POST, no x402 payment). The
/// escrow/payment only happens when the job poster selects a candidate.
pub struct TinyplaceJobApplyTool;

#[async_trait]
impl Tool for TinyplaceJobApplyTool {
    fn name(&self) -> &str {
        "tinyplace_job_apply"
    }

    fn description(&self) -> &str {
        "Submit a proposal (apply) to an open tiny.place job on behalf of the user. \
         Requires job_id. Optionally include a cover_letter, bid_amount (e.g. '450 USDC'), \
         estimated_delivery (e.g. '2 weeks'), and past_work URLs. \
         The candidate is always resolved from the user's wallet signer — it cannot \
         be supplied as an argument. Proposals are free (no payment required). \
         This is a write action: it submits an application on the user's behalf."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The tiny.place job ID to apply for."
                },
                "cover_letter": {
                    "type": "string",
                    "description": "Optional cover letter describing experience and fit for the role."
                },
                "bid_amount": {
                    "type": "string",
                    "description": "Optional bid amount, e.g. '450 USDC'."
                },
                "estimated_delivery": {
                    "type": "string",
                    "description": "Optional estimated delivery time, e.g. '2 weeks'."
                },
                "past_work": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of past work URLs or descriptions."
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let job_id = args
            .get("job_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("missing required parameter 'job_id'"))?;

        let cover_letter = args
            .get("cover_letter")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let bid_amount = args
            .get("bid_amount")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let estimated_delivery = args
            .get("estimated_delivery")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let past_work: Option<Vec<String>> = args
            .get("past_work")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value::<Vec<String>>(v.clone())
                    .map_err(|e| anyhow::anyhow!("invalid 'past_work' param: {e}"))
            })
            .transpose()?;

        log::debug!(
            "{LOG_PREFIX} tinyplace_job_apply job_id={job_id} \
             has_cover_letter={} has_bid={} has_delivery={} past_work_count={}",
            cover_letter.is_some(),
            bid_amount.is_some(),
            estimated_delivery.is_some(),
            past_work.as_ref().map(|v| v.len()).unwrap_or(0),
        );

        // Resolve candidate anti-spoof: always derived from the wallet signer.
        // The agent cannot supply a candidate arg — the signer is the source of truth.
        let client = global_state()
            .client()
            .await
            .map_err(|e| anyhow::anyhow!("tinyplace client unavailable: {e}"))?;

        let signer = client
            .http()
            .signer()
            .ok_or_else(|| anyhow::anyhow!("tiny.place signer unavailable; unlock your wallet"))?;

        // Candidate is always from the signer — not from tool arguments.
        let candidate = signer.agent_id();

        log::debug!("{LOG_PREFIX} tinyplace_job_apply candidate_resolved=true job_id={job_id}");

        let request = ProposalCreateRequest {
            candidate,
            cover_letter,
            bid_amount,
            estimated_delivery,
            past_work,
        };

        let result = client
            .jobs
            .apply(&job_id, &request)
            .await
            .map_err(|e| anyhow::anyhow!("{}", map_err(e)))?;

        log::debug!(
            "{LOG_PREFIX} tinyplace_job_apply success proposal_id={}",
            result.proposal_id
        );

        let output = serde_json::to_string(&result)
            .map_err(|e| anyhow::anyhow!("tinyplace serialise: {e}"))?;

        Ok(ToolResult::success(output))
    }

    fn permission_level(&self) -> PermissionLevel {
        // Write — submits a proposal on the user's behalf.
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        // POSTs a proposal to an external service.
        true
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        false
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};
    use serde_json::json;

    #[test]
    fn tool_metadata() {
        let tool = TinyplaceJobApplyTool;
        assert_eq!(tool.name(), "tinyplace_job_apply");
        assert_eq!(tool.permission_level(), PermissionLevel::Write);
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.external_effect());
        assert!(!tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn controller_tools_surface_core_tinyplace_actions() {
        let tools = all_tinyplace_agent_tools();
        let names: std::collections::HashSet<&str> = tools.iter().map(|tool| tool.name()).collect();

        for required in [
            "tinyplace_registry_register",
            "tinyplace_marketplace_buy_identity",
            "tinyplace_inbox_list",
            "tinyplace_signal_send_message",
            "tinyplace_groups_set_member_role",
            "tinyplace_bounties_approve",
            "tinyplace_jobs_select",
        ] {
            assert!(
                names.contains(required),
                "missing tiny.place controller tool `{required}`"
            );
        }

        let register = tools
            .iter()
            .find(|tool| tool.name() == "tinyplace_registry_register")
            .expect("registry register tool");
        assert_eq!(register.permission_level(), PermissionLevel::Write);
        assert!(register.external_effect());
        assert!(!register.is_concurrency_safe(&json!({})));

        let inbox = tools
            .iter()
            .find(|tool| tool.name() == "tinyplace_inbox_list")
            .expect("inbox list tool");
        assert_eq!(inbox.permission_level(), PermissionLevel::ReadOnly);
        assert!(!inbox.external_effect());
        assert!(inbox.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn signal_state_mutating_tools_are_write_external_effects() {
        let tools = all_tinyplace_agent_tools();

        for name in [
            "tinyplace_signal_provision",
            "tinyplace_signal_decrypt_message",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.name() == name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(tool.permission_level(), PermissionLevel::Write);
            assert!(tool.external_effect(), "{name} should prompt/audit");
            assert!(
                !tool.is_concurrency_safe(&json!({})),
                "{name} mutates Signal state"
            );
        }
    }

    #[test]
    fn controller_tool_parameters_come_from_controller_schema() {
        let tools = all_tinyplace_agent_tools();
        let resolve = tools
            .iter()
            .find(|tool| tool.name() == "tinyplace_directory_resolve")
            .expect("directory resolve tool");
        let schema = resolve.parameters_schema();
        let required = schema["required"].as_array().expect("required array");

        assert!(required.iter().any(|v| v.as_str() == Some("name")));
        assert_eq!(schema["properties"]["name"]["type"], "string");
    }

    #[test]
    fn optional_json_controller_params_remain_unrestricted() {
        let tools = all_tinyplace_agent_tools();
        let list_agents = tools
            .iter()
            .find(|tool| tool.name() == "tinyplace_directory_list_agents")
            .expect("directory list agents tool");
        let list_schema = list_agents.parameters_schema();
        assert!(
            list_schema["properties"]["params"].get("type").is_none(),
            "optional JSON params must not be emitted as null-only"
        );

        let jobs_apply = tools
            .iter()
            .find(|tool| tool.name() == "tinyplace_jobs_apply")
            .expect("jobs apply tool");
        let apply_schema = jobs_apply.parameters_schema();
        assert!(
            apply_schema["properties"]["pastWork"].get("type").is_none(),
            "optional JSON pastWork must not be emitted as null-only"
        );
    }

    #[test]
    fn parameters_schema_requires_job_id() {
        let schema = TinyplaceJobApplyTool.parameters_schema();
        let required = schema["required"].as_array().expect("required array");
        assert!(required.iter().any(|v| v.as_str() == Some("job_id")));

        // Candidate must NOT be in the schema — it's resolved server-side.
        let props = schema["properties"].as_object().expect("properties object");
        assert!(
            !props.contains_key("candidate"),
            "candidate must not be a tool argument (anti-spoof)"
        );
        assert!(props.contains_key("job_id"));
        assert!(props.contains_key("cover_letter"));
        assert!(props.contains_key("bid_amount"));
        assert!(props.contains_key("estimated_delivery"));
        assert!(props.contains_key("past_work"));
    }

    #[tokio::test]
    async fn missing_job_id_returns_error_before_client() {
        // Passes empty args — should fail with a clear error before
        // attempting any network call or client initialisation.
        let result = TinyplaceJobApplyTool.execute(json!({})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("job_id"),
            "error should mention 'job_id', got: {msg}"
        );
    }

    #[tokio::test]
    async fn missing_job_id_with_other_fields_still_errors() {
        let result = TinyplaceJobApplyTool
            .execute(json!({
                "cover_letter": "Great project",
                "bid_amount": "100 USDC"
            }))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("job_id"));
    }

    #[tokio::test]
    async fn blank_job_id_returns_error_before_client() {
        let result = TinyplaceJobApplyTool
            .execute(json!({ "job_id": "   " }))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("job_id"));
    }

    #[test]
    fn proposal_create_request_shape_from_tool_args() {
        // Verify the ProposalCreateRequest struct can be constructed as the
        // tool would produce it — candidate always from signer, never from args.
        let candidate = "agent_1abc".to_string();
        let request = ProposalCreateRequest {
            candidate: candidate.clone(),
            cover_letter: Some("I can do this".to_string()),
            bid_amount: Some("200 USDC".to_string()),
            estimated_delivery: Some("1 week".to_string()),
            past_work: Some(vec!["https://example.com/project".to_string()]),
        };
        assert_eq!(request.candidate, candidate);
        assert_eq!(request.cover_letter.as_deref(), Some("I can do this"));
        assert_eq!(request.bid_amount.as_deref(), Some("200 USDC"));
        assert_eq!(request.estimated_delivery.as_deref(), Some("1 week"));
        assert_eq!(request.past_work.as_ref().map(|v| v.len()), Some(1));
    }
}
