use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct GmailUnsubscribeTool;

#[async_trait]
impl Tool for GmailUnsubscribeTool {
    fn name(&self) -> &str {
        "gmail_unsubscribe"
    }

    fn description(&self) -> &str {
        "Initiates an unsubscribe request for an email sender. Requires the exact List-Unsubscribe header value and the sender's name/email to ask the user for confirmation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "sender": {
                    "type": "string",
                    "description": "The name and email address of the sender you are unsubscribing from."
                },
                "unsubscribe_link": {
                    "type": "string",
                    "description": "The exact URL or mailto link extracted from the List-Unsubscribe header."
                }
            },
            "required": ["sender", "unsubscribe_link"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sender = args.get("sender").and_then(|v| v.as_str()).unwrap_or("Unknown Sender");
        let link = args.get("unsubscribe_link").and_then(|v| v.as_str()).unwrap_or("");

        if link.is_empty() {
            return Ok(ToolResult::error("Cannot unsubscribe without a valid List-Unsubscribe link."));
        }

        // Return a structured JSON block indicating a Pending Action.
        // The React UI will intercept this exact payload.
        Ok(ToolResult::json(json!({
            "status": "pending_approval",
            "action": "unsubscribe",
            "metadata": {
                "sender": sender,
                "unsubscribe_link": link,
                "message": format!("The agent is requesting permission to unsubscribe you from: {}", sender)
            }
        })))
    }
}
