use crate::openhuman::tools::traits::{Tool, ToolResult};
use crate::openhuman::whatsapp_data::rpc as whatsapp_rpc;
use crate::openhuman::whatsapp_data::types::ListChatsRequest;
use async_trait::async_trait;
use serde_json::json;

pub struct WhatsAppDataListChatsTool;

#[async_trait]
impl Tool for WhatsAppDataListChatsTool {
    fn name(&self) -> &str {
        "whatsapp_data_list_chats"
    }

    fn description(&self) -> &str {
        "List WhatsApp chats stored locally on this device, newest activity \
         first. Use ONLY when the user asks about WhatsApp conversations or \
         which WhatsApp contacts/groups they have been talking to. Each chat \
         carries `chat_id`, `display_name`, `is_group`, `last_message_ts`, \
         and `message_count`; use those to drive `whatsapp_data_list_messages` \
         or `whatsapp_data_search_messages` afterwards. Returns provider \
         provenance so replies can cite WhatsApp."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional WhatsApp account JID. Omit to span every connected WhatsApp account."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum chats to return (default 50)."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Pagination offset (default 0)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][whatsapp_data] list_chats invoked");
        let req: ListChatsRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for whatsapp_data_list_chats: {e}"))?;
        let outcome = whatsapp_rpc::whatsapp_data_list_chats(req)
            .await
            .map_err(|e| anyhow::anyhow!("whatsapp_data_list_chats: {e}"))?;
        let chats = outcome.value;
        log::debug!(
            "[tool][whatsapp_data] list_chats returning count={}",
            chats.len()
        );
        let body = serde_json::to_string(&json!({
            "provider": "whatsapp",
            "count": chats.len(),
            "chats": chats,
        }))?;
        Ok(ToolResult::success(body))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

    #[test]
    fn metadata_advertises_whatsapp() {
        let tool = WhatsAppDataListChatsTool;
        assert_eq!(tool.name(), "whatsapp_data_list_chats");
        assert!(tool.description().contains("WhatsApp"));
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.is_concurrency_safe(&serde_json::Value::Null));
    }

    #[test]
    fn parameters_schema_is_object_with_optional_fields() {
        let schema = WhatsAppDataListChatsTool.parameters_schema();
        assert_eq!(schema["type"], "object");
        let props = &schema["properties"];
        for key in ["account_id", "limit", "offset"] {
            assert!(props.get(key).is_some(), "missing property {key}");
        }
        // No `required` array — every parameter is optional.
        assert!(schema.get("required").is_none());
    }

    #[tokio::test]
    async fn execute_rejects_invalid_args() {
        let tool = WhatsAppDataListChatsTool;
        let err = tool
            .execute(json!({ "limit": "not-a-number" }))
            .await
            .expect_err("expected invalid-args error");
        assert!(err.to_string().contains("whatsapp_data_list_chats"));
    }
}
