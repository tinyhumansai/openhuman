//! Twilio phone-call integration tool.
//!
//! **Scope**: CLI/RPC only — phone calls require explicit user action.
//!
//! **Endpoint**: `POST /agent-integrations/twilio/call`
//!
//! **Pricing** (fetched from backend):
//!   - Outbound calls: ~$0.03/min
//!   - Inbound calls:  ~$0.017/min
//!
//! The backend handles Twilio API credentials, billing, and rate limiting.

use super::IntegrationClient;
use crate::openhuman::integrations::ToolScope;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Makes outbound phone calls via the backend Twilio integration.
pub struct TwilioCallTool {
    client: Arc<IntegrationClient>,
}

#[derive(Debug, Deserialize)]
struct TwilioCallResponse {
    #[serde(rename = "callSid")]
    call_sid: String,
    status: String,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

impl TwilioCallTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for TwilioCallTool {
    fn name(&self) -> &str {
        "twilio_call"
    }

    fn description(&self) -> &str {
        "Make an outbound phone call via Twilio. Requires at least one of: \
         a text message to speak, raw TwiML markup, or a TwiML URL. \
         The call is placed immediately and the result includes the call SID \
         and initial status. Cost is billed per minute by the backend."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Phone number to call in E.164 format (e.g. +14155551234)"
                },
                "message": {
                    "type": "string",
                    "description": "Plain text message to speak on the call (uses TTS)"
                },
                "twiml": {
                    "type": "string",
                    "description": "Raw TwiML XML to control call flow (advanced)"
                },
                "url": {
                    "type": "string",
                    "description": "URL that returns TwiML for call flow (advanced)"
                }
            },
            "required": ["to"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn scope(&self) -> ToolScope {
        ToolScope::CliRpcOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: to"))?;

        if to.trim().is_empty() {
            return Ok(ToolResult::error("Phone number 'to' cannot be empty"));
        }

        let message = args.get("message").and_then(|v| v.as_str());
        let twiml = args.get("twiml").and_then(|v| v.as_str());
        let url = args.get("url").and_then(|v| v.as_str());

        if message.is_none() && twiml.is_none() && url.is_none() {
            return Ok(ToolResult::error(
                "At least one of 'message', 'twiml', or 'url' must be provided",
            ));
        }

        let mut body = json!({ "to": to });
        if let Some(m) = message {
            body["message"] = json!(m);
        }
        if let Some(t) = twiml {
            body["twiml"] = json!(t);
        }
        if let Some(u) = url {
            body["url"] = json!(u);
        }

        let redacted = if to.len() > 4 {
            format!(
                "{}***{}",
                &to[..to.char_indices().nth(2).map_or(2, |(i, _)| i)],
                &to[to
                    .char_indices()
                    .rev()
                    .nth(3)
                    .map_or(to.len().saturating_sub(4), |(i, _)| i)..]
            )
        } else {
            "****".to_string()
        };
        tracing::info!("[twilio_call] calling {}", redacted);

        match self
            .client
            .post::<TwilioCallResponse>("/agent-integrations/twilio/call", &body)
            .await
        {
            Ok(resp) => {
                let output = format!(
                    "Call placed successfully.\n\
                     Call SID: {}\n\
                     Status: {}\n\
                     Cost: ${:.4}",
                    resp.call_sid, resp.status, resp.cost_usd
                );
                Ok(ToolResult::success(output))
            }
            Err(e) => Ok(ToolResult::error(format!("Twilio call failed: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "twilio_tests.rs"]
mod tests;
