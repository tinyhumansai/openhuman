//! LLM-callable agent tools for the `tinyplace` domain.
//!
//! Exposes write actions (job proposal submission) to the agent tool-call
//! pipeline. The candidate is always resolved server-side from the wallet
//! signer (`signer.agent_id()`) to prevent impersonation — it is never
//! accepted as a tool argument.

use async_trait::async_trait;
use serde_json::json;
use tinyplace::types::ProposalCreateRequest;

use crate::openhuman::tinyplace::ops::{global_state, map_err};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

const LOG_PREFIX: &str = "[tinyplace][tool]";

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
