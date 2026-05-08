use crate::openhuman::tools::traits::{Tool, ToolResult};
use crate::openhuman::whatsapp_data::rpc as whatsapp_rpc;
use crate::openhuman::whatsapp_data::types::ListMessagesRequest;
use async_trait::async_trait;
use serde_json::json;

pub struct WhatsAppDataListMessagesTool;

#[async_trait]
impl Tool for WhatsAppDataListMessagesTool {
    fn name(&self) -> &str {
        "whatsapp_data_list_messages"
    }

    fn description(&self) -> &str {
        "Return WhatsApp messages for one chat, ordered oldest-first within \
         the requested time window. USE THIS for any WhatsApp request scoped \
         to a specific chat — summarisation, action-item extraction, \
         quoting, or 'show me the last N messages with <person>'. ALSO use \
         this (paired with `since_ts` from `current_time` minus N hours) for \
         time-window reads like 'what did <person> message me in the last 3 \
         hours' AFTER resolving the chat via `whatsapp_data_list_chats`. The \
         `chat_id` arg is required — get it from `whatsapp_data_list_chats` \
         (or `whatsapp_data_search_messages`); optionally bound the range \
         with `since_ts` / `until_ts` (Unix seconds). Do NOT use \
         `whatsapp_data_search_messages` for time-only queries — that tool \
         is keyword-based, not time-based. Returns provider provenance so \
         replies can cite WhatsApp."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chat_id": {
                    "type": "string",
                    "description": "WhatsApp chat JID, e.g. `1234567890@c.us` for a contact or `<id>@g.us` for a group. Required."
                },
                "account_id": {
                    "type": "string",
                    "description": "Optional WhatsApp account JID. Omit to span every connected account."
                },
                "since_ts": {
                    "type": "integer",
                    "description": "Lower bound (Unix seconds, inclusive) on message timestamp."
                },
                "until_ts": {
                    "type": "integer",
                    "description": "Upper bound (Unix seconds, inclusive) on message timestamp."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum messages to return (default 100)."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Pagination offset (default 0)."
                }
            },
            "required": ["chat_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][whatsapp_data] list_messages invoked");
        let req: ListMessagesRequest = serde_json::from_value(args).map_err(|e| {
            log::debug!("[tool][whatsapp_data] list_messages invalid_args error={e}");
            anyhow::anyhow!("invalid arguments for whatsapp_data_list_messages: {e}")
        })?;
        log::debug!(
            "[tool][whatsapp_data] list_messages args has_account={} has_chat=true limit={:?} offset={:?} has_since={} has_until={}",
            req.account_id.is_some(),
            req.limit,
            req.offset,
            req.since_ts.is_some(),
            req.until_ts.is_some(),
        );
        let outcome = whatsapp_rpc::whatsapp_data_list_messages(req)
            .await
            .map_err(|e| {
                log::warn!("[tool][whatsapp_data] list_messages rpc_error error={e}");
                anyhow::anyhow!("whatsapp_data_list_messages: {e}")
            })?;
        let messages = outcome.value;
        log::debug!(
            "[tool][whatsapp_data] list_messages returning count={}",
            messages.len()
        );
        let body = serde_json::to_string(&json!({
            "provider": "whatsapp",
            "count": messages.len(),
            "messages": messages,
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
        let tool = WhatsAppDataListMessagesTool;
        assert_eq!(tool.name(), "whatsapp_data_list_messages");
        assert!(tool.description().contains("WhatsApp"));
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.is_concurrency_safe(&serde_json::Value::Null));
    }

    #[test]
    fn parameters_schema_requires_chat_id() {
        let schema = WhatsAppDataListMessagesTool.parameters_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"]
            .as_array()
            .expect("required array present");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["chat_id"]);
    }

    #[tokio::test]
    async fn execute_rejects_missing_chat_id() {
        let tool = WhatsAppDataListMessagesTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("expected missing chat_id error");
        assert!(err.to_string().contains("whatsapp_data_list_messages"));
    }
}
