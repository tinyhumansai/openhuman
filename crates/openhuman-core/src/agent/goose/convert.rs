use anyhow::{anyhow, Result};
use goose_provider_types::conversation::{
    message::{Message as GooseMessage, MessageContent},
    Conversation,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock as McpContent};
use tinyinference::{
    message::{AssistantMessage, ContentBlock, Message, ToolMessage},
    tool::{ToolCall as TinyToolCall, ToolSchema},
};

use crate::{
    agent::messages::{ChatMessage, ConversationMessage, ToolResultMessage},
    inference::provider::ToolCall,
};

const META_OPERATION: &str = "openhuman_adapter";
const META_ROLE: &str = "role";
const META_EXTRA: &str = "extra_metadata";

fn mark_chat_metadata(message: &mut GooseMessage, chat: &ChatMessage) {
    message
        .metadata
        .set_operation_note(META_OPERATION, META_ROLE, chat.role.clone().into());
    if let Some(extra) = chat.extra_metadata.clone() {
        message
            .metadata
            .set_operation_note(META_OPERATION, META_EXTRA, extra);
    }
    message.id = chat.id.clone();
}

fn call_to_rmcp(call: &ToolCall) -> Result<CallToolRequestParams, rmcp::model::ErrorData> {
    let arguments: serde_json::Value = serde_json::from_str(&call.arguments).map_err(|error| {
        rmcp::model::ErrorData::invalid_params(
            format!("invalid arguments for '{}': {error}", call.name),
            None,
        )
    })?;
    let arguments = arguments.as_object().cloned().ok_or_else(|| {
        rmcp::model::ErrorData::invalid_params(
            format!("arguments for '{}' must be a JSON object", call.name),
            None,
        )
    })?;
    Ok(CallToolRequestParams::new(call.name.clone()).with_arguments(arguments))
}

pub(super) fn openhuman_to_goose(messages: &[ConversationMessage]) -> Result<Conversation> {
    let mut out = Vec::new();
    for message in messages {
        match message {
            ConversationMessage::Chat(chat) => {
                let mut goose = match chat.role.as_str() {
                    "assistant" => GooseMessage::assistant().with_text(&chat.content),
                    _ => GooseMessage::user().with_text(&chat.content),
                };
                mark_chat_metadata(&mut goose, chat);
                out.push(goose);
            }
            ConversationMessage::AssistantToolCalls {
                text,
                tool_calls,
                reasoning_content,
                extra_metadata,
            } => {
                let mut goose = GooseMessage::assistant();
                if let Some(reasoning) = reasoning_content {
                    goose = goose.with_thinking(reasoning, "");
                }
                if let Some(text) = text {
                    goose = goose.with_text(text);
                }
                if let Some(extra) = extra_metadata.clone() {
                    goose
                        .metadata
                        .set_operation_note(META_OPERATION, META_EXTRA, extra);
                }
                for call in tool_calls {
                    goose = goose.with_tool_request(call.id.clone(), call_to_rmcp(call));
                }
                out.push(goose);
            }
            ConversationMessage::ToolResults(results) => {
                let mut goose = GooseMessage::user();
                for result in results {
                    goose = goose.with_tool_response(
                        result.tool_call_id.clone(),
                        Ok(CallToolResult::success(vec![McpContent::text(
                            result.content.clone(),
                        )])),
                    );
                }
                out.push(goose);
            }
        }
    }
    Ok(Conversation::new_unvalidated(out))
}

fn extra_metadata(message: &GooseMessage) -> Option<serde_json::Value> {
    message
        .metadata
        .operation_note(META_OPERATION, META_EXTRA)
        .cloned()
}

fn marked_role(message: &GooseMessage) -> Option<&str> {
    message
        .metadata
        .operation_note(META_OPERATION, META_ROLE)
        .and_then(serde_json::Value::as_str)
}

pub(super) fn rmcp_result_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn goose_to_openhuman(conversation: &Conversation) -> Vec<ConversationMessage> {
    let mut out = Vec::new();
    for message in conversation.messages() {
        let requests = message
            .content
            .iter()
            .filter_map(MessageContent::as_tool_request)
            .collect::<Vec<_>>();
        let responses = message
            .content
            .iter()
            .filter_map(MessageContent::as_tool_response)
            .collect::<Vec<_>>();
        if !requests.is_empty() {
            let tool_calls = requests
                .into_iter()
                .map(|request| match &request.tool_call {
                    Ok(call) => ToolCall {
                        id: request.id.clone(),
                        name: call.name.to_string(),
                        arguments: serde_json::Value::Object(
                            call.arguments.clone().unwrap_or_default(),
                        )
                        .to_string(),
                        extra_content: None,
                    },
                    Err(error) => ToolCall {
                        id: request.id.clone(),
                        name: String::new(),
                        arguments: error.message.to_string(),
                        extra_content: None,
                    },
                })
                .collect();
            let text = message.as_concat_text();
            let reasoning = message
                .content
                .iter()
                .filter_map(MessageContent::as_thinking)
                .map(|thinking| thinking.thinking.as_str())
                .collect::<Vec<_>>()
                .join("");
            out.push(ConversationMessage::AssistantToolCalls {
                text: (!text.is_empty()).then_some(text),
                tool_calls,
                reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                extra_metadata: extra_metadata(message),
            });
            continue;
        }
        if !responses.is_empty() {
            let results = responses
                .into_iter()
                .map(|response| ToolResultMessage {
                    tool_call_id: response.id.clone(),
                    content: match &response.tool_result {
                        Ok(result) => rmcp_result_text(result),
                        Err(error) => error.message.to_string(),
                    },
                })
                .collect();
            out.push(ConversationMessage::ToolResults(results));
            continue;
        }

        let role = marked_role(message).unwrap_or(match message.role {
            rmcp::model::Role::Assistant => "assistant",
            rmcp::model::Role::User => "user",
        });
        let mut chat = match role {
            "system" => ChatMessage::system(message.as_concat_text()),
            "assistant" => ChatMessage::assistant(message.as_concat_text()),
            "tool" => ChatMessage::tool(message.as_concat_text()),
            _ => ChatMessage::user(message.as_concat_text()),
        };
        chat.id = message.id.clone();
        chat.extra_metadata = extra_metadata(message);
        out.push(ConversationMessage::Chat(chat));
    }
    out
}

pub(super) fn goose_to_model_messages(conversation: &Conversation) -> Vec<Message> {
    let mut out = Vec::new();
    for message in conversation.agent_visible_messages() {
        let role = marked_role(&message);
        if role == Some("system") {
            out.push(Message::system(message.as_concat_text()));
            continue;
        }
        let requests = message
            .content
            .iter()
            .filter_map(MessageContent::as_tool_request)
            .filter_map(|request| {
                let call = request.tool_call.as_ref().ok()?;
                Some(TinyToolCall::new(
                    request.id.clone(),
                    call.name.to_string(),
                    serde_json::Value::Object(call.arguments.clone().unwrap_or_default()),
                ))
            })
            .collect::<Vec<_>>();
        let responses = message
            .content
            .iter()
            .filter_map(MessageContent::as_tool_response)
            .collect::<Vec<_>>();
        if !responses.is_empty() {
            for response in responses {
                let content = match &response.tool_result {
                    Ok(result) => rmcp_result_text(result),
                    Err(error) => error.message.to_string(),
                };
                out.push(Message::Tool(ToolMessage {
                    tool_call_id: response.id.clone(),
                    content: vec![ContentBlock::Text(content)],
                    trusted_verbatim: false,
                    artifact: None,
                }));
            }
        } else if message.role == rmcp::model::Role::Assistant {
            let mut content = Vec::new();
            for block in &message.content {
                if let Some(thinking) = block.as_thinking() {
                    content.push(ContentBlock::thinking(&thinking.thinking));
                } else if let Some(text) = block.as_text() {
                    content.push(ContentBlock::Text(text.to_string()));
                }
            }
            out.push(Message::Assistant(AssistantMessage {
                id: message.id.clone(),
                content,
                tool_calls: requests,
                usage: None,
            }));
        } else {
            out.push(Message::user(message.as_concat_text()));
        }
    }
    out
}

pub(super) fn rmcp_tools_to_tiny(tools: &[rmcp::model::Tool]) -> Vec<ToolSchema> {
    tools
        .iter()
        .map(|tool| {
            ToolSchema::new(
                tool.name.to_string(),
                tool.description
                    .as_deref()
                    .unwrap_or("No description supplied"),
                serde_json::Value::Object((*tool.input_schema).clone()),
            )
        })
        .collect()
}

pub(super) fn tiny_response_to_goose(
    response: &tinyinference::model::ModelResponse,
) -> GooseMessage {
    let mut message = GooseMessage::assistant();
    for block in &response.message.content {
        match block {
            ContentBlock::Text(text) => message = message.with_text(text),
            ContentBlock::Thinking { text, signature } => {
                message = message.with_thinking(text, signature.as_deref().unwrap_or(""));
            }
            ContentBlock::RedactedThinking { data } => {
                message = message.with_redacted_thinking(data);
            }
            _ => {}
        }
    }
    for call in &response.message.tool_calls {
        let rmcp_call = call
            .arguments
            .as_object()
            .cloned()
            .map(|arguments| {
                CallToolRequestParams::new(call.name.clone()).with_arguments(arguments)
            })
            .ok_or_else(|| {
                rmcp::model::ErrorData::invalid_params(
                    format!("arguments for '{}' must be an object", call.name),
                    None,
                )
            });
        message = message.with_tool_request(call.id.clone(), rmcp_call);
    }
    message
}

pub(super) fn final_text(conversation: &Conversation) -> Option<String> {
    let last = conversation.last()?;
    (last.role == rmcp::model::Role::Assistant)
        .then(|| last.as_concat_text())
        .filter(|text| !text.trim().is_empty())
}

pub(super) fn ensure_nonempty_kickoff(conversation: &Conversation) -> Result<()> {
    if conversation.messages().iter().any(|message| {
        message.role == rmcp::model::Role::User
            && !message.is_tool_response()
            && !message.as_concat_text().trim().is_empty()
    }) {
        Ok(())
    } else {
        Err(anyhow!("Goose conversation has no OpenHuman user kickoff"))
    }
}
