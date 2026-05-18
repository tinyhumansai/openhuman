use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use async_trait::async_trait;

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
        serde_json::json!({
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

    // Intentionally NOT marked external_effect=true in v1.
    //
    // `execute()` below does NOT perform the unsubscribe itself —
    // it returns a `pending_approval` JSON payload that the React
    // UI intercepts and gates with its own legacy confirmation
    // flow. Marking this tool external_effect=true would route the
    // call through the new `ApprovalGate` AND the legacy UI prompt,
    // so users would see two consecutive approval dialogs while the
    // real side effect still lived outside core enforcement.
    //
    // Follow-up #1339-v3: move the actual outbound unsubscribe
    // into `execute()`, retire the legacy UI prompt, and then flip
    // `external_effect = true` here.

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sender = args
            .get("sender")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Sender");

        let redacted_sender = if let Some(idx) = sender.find('@') {
            format!("***{}", &sender[idx..])
        } else {
            "***".to_string()
        };
        tracing::debug!("GMAIL_UNSUBSCRIBE:ENTRY sender={}", redacted_sender);

        let link = args
            .get("unsubscribe_link")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if link.is_empty() {
            tracing::debug!("GMAIL_UNSUBSCRIBE:VALIDATION:EMPTY_LINK");
            return Ok(ToolResult::error(
                "Cannot unsubscribe without a valid List-Unsubscribe link.",
            ));
        }

        tracing::debug!(
            "GMAIL_UNSUBSCRIBE:PENDING_APPROVAL sender={} action=unsubscribe status=pending_approval",
            redacted_sender
        );

        // Return a structured JSON block indicating a Pending Action.
        // The React UI will intercept this exact payload.
        Ok(ToolResult::json(serde_json::json!({
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolContent;

    #[tokio::test]
    async fn test_gmail_unsubscribe_valid() {
        let tool = GmailUnsubscribeTool;
        let result = tool
            .execute(serde_json::json!({
                "sender": "marketing@example.com",
                "unsubscribe_link": "https://example.com/unsub"
            }))
            .await
            .unwrap();

        assert!(!result.is_error);
        let mut has_json = false;
        for content in result.content {
            if let ToolContent::Json { data: value } = content {
                assert_eq!(value["status"].as_str().unwrap(), "pending_approval");
                assert_eq!(value["action"].as_str().unwrap(), "unsubscribe");
                assert_eq!(
                    value["metadata"]["sender"].as_str().unwrap(),
                    "marketing@example.com"
                );
                assert_eq!(
                    value["metadata"]["unsubscribe_link"].as_str().unwrap(),
                    "https://example.com/unsub"
                );
                has_json = true;
            }
        }
        assert!(has_json, "Expected JSON result");
    }

    #[tokio::test]
    async fn test_gmail_unsubscribe_empty_link() {
        let tool = GmailUnsubscribeTool;
        let result = tool
            .execute(serde_json::json!({
                "sender": "marketing@example.com",
                "unsubscribe_link": ""
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result
            .text()
            .contains("without a valid List-Unsubscribe link"));
    }

    #[tokio::test]
    async fn test_gmail_unsubscribe_missing_link() {
        let tool = GmailUnsubscribeTool;
        let result = tool
            .execute(serde_json::json!({
                "sender": "marketing@example.com"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result
            .text()
            .contains("without a valid List-Unsubscribe link"));
    }
}
