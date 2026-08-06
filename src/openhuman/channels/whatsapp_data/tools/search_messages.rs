use crate::core::event_bus::request_native_global;
use crate::openhuman::channels::whatsapp_data::methods;
use crate::openhuman::channels::whatsapp_data::tools::{is_handler_absent, UNAVAILABLE_NOTE};
use crate::openhuman::channels::whatsapp_data::types::{SearchMessagesRequest, WhatsAppMessage};
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct WhatsAppDataSearchMessagesTool;

#[async_trait]
impl Tool for WhatsAppDataSearchMessagesTool {
    fn name(&self) -> &str {
        "whatsapp_data_search_messages"
    }

    fn description(&self) -> &str {
        "Case-insensitive substring search across stored WhatsApp messages, \
         newest-first. Matches BOTH the message body AND the sender name, so \
         a query of 'Alice' returns Alice's own messages even when the body \
         does not contain the word 'Alice'. USE THIS for keyword lookups \
         (specific words, phrases, project names, URLs in messages) and for \
         'what did <person> say about <topic>' style intents. \
         Do NOT use this for time-window queries like 'what did <person> say \
         in the last 3 hours' — those go through `whatsapp_data_list_chats` \
         to resolve the chat, then `whatsapp_data_list_messages` with \
         `since_ts`. Optionally narrow with `chat_id` and/or `account_id`. \
         Returns provider provenance so replies can cite WhatsApp."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Substring to match against message bodies (case-insensitive). Required."
                },
                "chat_id": {
                    "type": "string",
                    "description": "Optional WhatsApp chat JID to scope the search."
                },
                "account_id": {
                    "type": "string",
                    "description": "Optional WhatsApp account JID. Omit to span every connected account."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum messages to return (default 20)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][whatsapp_data] search_messages invoked");
        let req: SearchMessagesRequest = serde_json::from_value(args).map_err(|e| {
            log::debug!("[tool][whatsapp_data] search_messages invalid_args error={e}");
            anyhow::anyhow!("invalid arguments for whatsapp_data_search_messages: {e}")
        })?;
        log::debug!(
            "[tool][whatsapp_data] search_messages args has_account={} has_chat={} limit={:?} query_len={}",
            req.account_id.is_some(),
            req.chat_id.is_some(),
            req.limit,
            req.query.len(),
        );
        let messages: Vec<WhatsAppMessage> =
            match request_native_global(methods::SEARCH_MESSAGES, req).await {
                Ok(messages) => messages,
                Err(e) if is_handler_absent(&e) => {
                    log::debug!(
                        "[tool][whatsapp_data] search_messages handler_absent — degrading ({e})"
                    );
                    let body = serde_json::to_string(&json!({
                        "provider": "whatsapp",
                        "count": 0,
                        "messages": [],
                        "note": UNAVAILABLE_NOTE,
                    }))?;
                    return Ok(ToolResult::success(body));
                }
                Err(e) => {
                    log::warn!("[tool][whatsapp_data] search_messages bridge_error error={e}");
                    return Err(anyhow::anyhow!("whatsapp_data_search_messages: {e}"));
                }
            };
        log::debug!(
            "[tool][whatsapp_data] search_messages returning count={}",
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
        let tool = WhatsAppDataSearchMessagesTool;
        assert_eq!(tool.name(), "whatsapp_data_search_messages");
        assert!(tool.description().contains("WhatsApp"));
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.is_concurrency_safe(&serde_json::Value::Null));
    }

    #[test]
    fn parameters_schema_requires_query() {
        let schema = WhatsAppDataSearchMessagesTool.parameters_schema();
        let required = schema["required"]
            .as_array()
            .expect("required array present");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["query"]);
    }

    #[tokio::test]
    async fn execute_rejects_missing_query() {
        let tool = WhatsAppDataSearchMessagesTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("expected missing query error");
        assert!(err.to_string().contains("whatsapp_data_search_messages"));
    }

    #[tokio::test]
    async fn execute_degrades_gracefully_without_shell_handler() {
        let tool = WhatsAppDataSearchMessagesTool;
        let result = tool
            .execute(json!({ "query": "hello" }))
            .await
            .expect("degradation must succeed, not error");
        let body: serde_json::Value =
            serde_json::from_str(&result.text()).expect("tool body is JSON");
        assert_eq!(body["provider"], "whatsapp");
        assert_eq!(body["count"], 0);
        assert_eq!(body["note"], UNAVAILABLE_NOTE);
    }
}
