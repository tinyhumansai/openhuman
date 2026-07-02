//! Conversions between openhuman's flat [`ChatMessage`]/[`ToolSpec`]/[`ToolCall`]
//! wire types and the `tinyagents` harness' rich [`Message`]/[`ToolSchema`]/
//! [`TaToolCall`] equivalents (issue #4249).
//!
//! The two sides model the same concepts with different shapes:
//!
//! - openhuman `ChatMessage` is `{ role: String, content: String }` — tool
//!   calls and tool-result correlation ids are not first-class fields; the
//!   legacy loop threads them through provider-native encoding instead.
//! - `tinyagents::harness::message::Message` is a typed enum
//!   (`System`/`User`/`Assistant`/`Tool`) whose `Assistant` arm carries
//!   structured `tool_calls` and whose `Tool` arm carries a `tool_call_id`.
//!
//! These helpers bridge the seed history into the harness and the harness'
//! resulting transcript back out, so a turn can run on the `tinyagents`
//! agent-loop while callers keep speaking openhuman's `ChatMessage` vocabulary.

use tinyagents::harness::message::{
    AssistantMessage, ContentBlock, Message, SystemMessage, ToolMessage, UserMessage,
};
use tinyagents::harness::tool::{ToolCall as TaToolCall, ToolSchema};

use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage, ToolResultMessage};
use crate::openhuman::tools::ToolSpec;

/// Key under which a thinking model's `reasoning_content` is stashed on an
/// assistant [`Message`]'s content blocks (as a [`ContentBlock::ProviderExtension`])
/// and echoed back through openhuman [`ChatMessage::extra_metadata`]. tinyagents'
/// `AssistantMessage` has no reasoning channel of its own, so we carry it here so
/// it survives the round-trip and can be replayed verbatim on the next turn
/// (thinking-mode providers reject a multi-turn request that drops it).
pub(super) const REASONING_EXT_KEY: &str = "reasoning_content";

/// Build the [`ContentBlock`] that stashes a response's `reasoning_content` on an
/// assistant message, if any.
pub(super) fn reasoning_content_block(reasoning: Option<&str>) -> Option<ContentBlock> {
    let reasoning = reasoning?;
    // Store verbatim (only gate on non-empty after a trim): thinking-mode
    // providers validate the prior reasoning block byte-for-byte on a resumed
    // multi-turn request, so trimming boundary whitespace could break replay.
    (!reasoning.trim().is_empty()).then(|| {
        ContentBlock::ProviderExtension(serde_json::json!({ REASONING_EXT_KEY: reasoning }))
    })
}

/// Recover the stashed `reasoning_content` from an assistant message's content
/// blocks (see [`reasoning_content_block`]).
fn reasoning_from_content(content: &[ContentBlock]) -> Option<String> {
    content.iter().find_map(|block| match block {
        ContentBlock::ProviderExtension(value) => value
            .get(REASONING_EXT_KEY)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        _ => None,
    })
}

/// The `extra_metadata` an assistant [`ChatMessage`] should carry so a stashed
/// `reasoning_content` replays on the next provider request.
fn reasoning_extra_metadata(content: &[ContentBlock]) -> Option<serde_json::Value> {
    reasoning_from_content(content)
        .map(|reasoning| serde_json::json!({ REASONING_EXT_KEY: reasoning }))
}

/// Convert one openhuman [`ChatMessage`] into a harness [`Message`].
///
/// Role strings map onto the typed arms. A seeded **native** tool round is
/// serialized by [`NativeToolDispatcher::to_provider_messages`] as a
/// `{ "content", "tool_calls" }` assistant envelope followed by
/// `{ "tool_call_id", "content" }` tool envelopes; we unwrap those back into the
/// structured [`AssistantMessage::tool_calls`] / [`ToolMessage::tool_call_id`]
/// the harness needs. Without this, the seeded assistant loses its tool calls
/// while the following tool rows survive, so the harness re-sends orphan `tool`
/// messages and native providers reject the request (`assistant message with
/// 'tool_calls' must be followed by tool messages`). A plain assistant/tool
/// message that isn't an envelope maps straight through as text.
pub(super) fn chat_message_to_message(msg: &ChatMessage) -> Message {
    let text = msg.content.clone();
    match msg.role.as_str() {
        "system" => Message::System(SystemMessage {
            content: vec![ContentBlock::Text(text)],
        }),
        "assistant" => {
            // Restore any `reasoning_content` stashed on the persisted message so a
            // multi-turn thinking-mode conversation replays it verbatim (see
            // [`reasoning_content_block`]).
            let reasoning = msg
                .extra_metadata
                .as_ref()
                .and_then(|meta| meta.get(REASONING_EXT_KEY))
                .and_then(serde_json::Value::as_str);
            if let Some((inner, tool_calls)) = parse_native_assistant_envelope(&text) {
                let mut content = vec![ContentBlock::Text(inner)];
                content.extend(reasoning_content_block(reasoning));
                Message::Assistant(AssistantMessage {
                    id: msg.id.clone(),
                    content,
                    tool_calls,
                    usage: None,
                })
            } else {
                let mut content = vec![ContentBlock::Text(text)];
                content.extend(reasoning_content_block(reasoning));
                Message::Assistant(AssistantMessage {
                    id: msg.id.clone(),
                    content,
                    tool_calls: Vec::new(),
                    usage: None,
                })
            }
        }
        "tool" => {
            // Prefer the envelope's `tool_call_id` (the native seed shape); fall
            // back to the message id, then an empty id for a bare tool message.
            let (tool_call_id, content) = parse_native_tool_envelope(&text)
                .unwrap_or_else(|| (msg.id.clone().unwrap_or_default(), text.clone()));
            Message::Tool(ToolMessage {
                tool_call_id,
                content: vec![ContentBlock::Text(content)],
            })
        }
        // "user" and any unrecognized role default to a user turn — the safest
        // mapping for a free-form inbound message.
        _ => Message::User(UserMessage {
            content: vec![ContentBlock::Text(text)],
        }),
    }
}

/// Parse a native assistant tool-call envelope (`{ "content", "tool_calls" }`, as
/// [`NativeToolDispatcher::to_provider_messages`] emits) back into its inner
/// visible text and structured [`TaToolCall`]s. Returns `None` when `text` is not
/// such an envelope (plain assistant prose), so the caller can fall back to text.
fn parse_native_assistant_envelope(text: &str) -> Option<(String, Vec<TaToolCall>)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    let calls_val = obj.get("tool_calls")?;
    // Require a non-empty, parseable tool-call array so ordinary JSON-looking
    // assistant prose isn't misread as a tool round.
    if !calls_val.as_array().is_some_and(|a| !a.is_empty()) {
        return None;
    }
    let oh_calls: Vec<crate::openhuman::inference::provider::ToolCall> =
        serde_json::from_value(calls_val.clone()).ok()?;
    if oh_calls.is_empty() {
        return None;
    }
    let inner = obj
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    Some((inner, oh_calls.iter().map(oh_call_to_ta_call).collect()))
}

/// Parse a native tool-result envelope (`{ "tool_call_id", "content" }`) back into
/// its correlation id and payload. Returns `None` for a bare tool message.
fn parse_native_tool_envelope(text: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    let id = obj.get("tool_call_id")?.as_str()?.to_string();
    let content = obj
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    Some((id, content))
}

/// Inverse of [`ta_call_to_oh_call`]: rebuild a harness [`TaToolCall`] from an
/// openhuman [`ToolCall`] (whose `arguments` is a serialized JSON string).
fn oh_call_to_ta_call(oh: &crate::openhuman::inference::provider::ToolCall) -> TaToolCall {
    TaToolCall {
        id: oh.id.clone(),
        name: oh.name.clone(),
        arguments: serde_json::from_str(&oh.arguments).unwrap_or(serde_json::Value::Null),
    }
}

/// Convert a seed history into the harness `input` transcript.
pub(super) fn history_to_messages(history: &[ChatMessage]) -> Vec<Message> {
    history.iter().map(chat_message_to_message).collect()
}

/// Convert a harness [`Message`] back into an openhuman [`ChatMessage`].
///
/// Assistant tool calls are flattened to their text (the loop already executed
/// them and appended `Tool` result messages), and a tool message preserves its
/// correlation id on [`ChatMessage::id`] so downstream persistence keeps it.
pub(super) fn message_to_chat_message(msg: &Message) -> ChatMessage {
    match msg {
        Message::System(_) => ChatMessage::system(msg.text()),
        Message::User(_) => ChatMessage::user(msg.text()),
        Message::Assistant(a) => {
            let mut cm = ChatMessage::assistant(msg.text());
            cm.extra_metadata = reasoning_extra_metadata(&a.content);
            cm
        }
        Message::Tool(t) => {
            let mut cm = ChatMessage::tool(msg.text());
            cm.id = Some(t.tool_call_id.clone());
            cm
        }
    }
}

/// Convert a harness transcript back into openhuman history.
pub(super) fn messages_to_history(messages: &[Message]) -> Vec<ChatMessage> {
    messages.iter().map(message_to_chat_message).collect()
}

/// Convert one harness [`Message`] into a [`ChatMessage`] for a **native**
/// tool-calling provider request, preserving the structure the provider needs to
/// round-trip a tool round: an assistant turn that made tool calls is encoded as
/// the `{ "content", "tool_calls" }` JSON envelope (matching the dispatcher's
/// native `to_provider_messages`), and a tool result as `{ "tool_call_id",
/// "content" }`. Without this the provider sees an assistant with no `tool_calls`
/// followed by an orphan tool message and drops the round — breaking multi-turn
/// native tool calling (e.g. the orchestrator's `spawn_parallel_agents` →
/// synthesis hop).
pub(super) fn message_to_native_chat_message(msg: &Message) -> ChatMessage {
    match msg {
        Message::System(_) => ChatMessage::system(msg.text()),
        Message::User(_) => ChatMessage::user(msg.text()),
        Message::Assistant(a) if !a.tool_calls.is_empty() => {
            let tool_calls: Vec<_> = a.tool_calls.iter().map(ta_call_to_oh_call).collect();
            let payload = serde_json::json!({
                "content": msg.text(),
                "tool_calls": tool_calls,
            });
            let mut cm = ChatMessage::assistant(payload.to_string());
            cm.extra_metadata = reasoning_extra_metadata(&a.content);
            cm
        }
        Message::Assistant(a) => {
            let mut cm = ChatMessage::assistant(msg.text());
            cm.extra_metadata = reasoning_extra_metadata(&a.content);
            cm
        }
        Message::Tool(t) => {
            let payload = serde_json::json!({
                "tool_call_id": t.tool_call_id,
                "content": msg.text(),
            });
            let mut cm = ChatMessage::tool(payload.to_string());
            cm.id = Some(t.tool_call_id.clone());
            cm
        }
    }
}

/// Convert a harness transcript into the **typed** [`ConversationMessage`] shape
/// the chat session persists, preserving assistant tool-call structure
/// (`AssistantToolCalls`) and tool results (`ToolResults`) — unlike
/// [`messages_to_history`], which flattens tool calls to text.
///
/// Consecutive `Tool` messages are coalesced into one `ToolResults` batch (the
/// shape a single assistant tool-call round produces), matching the legacy
/// `turn_engine_adapter` persistence.
pub(super) fn messages_to_conversation(messages: &[Message]) -> Vec<ConversationMessage> {
    let mut out: Vec<ConversationMessage> = Vec::new();
    let mut pending: Vec<ToolResultMessage> = Vec::new();

    fn flush(out: &mut Vec<ConversationMessage>, pending: &mut Vec<ToolResultMessage>) {
        if !pending.is_empty() {
            out.push(ConversationMessage::ToolResults(std::mem::take(pending)));
        }
    }

    for msg in messages {
        match msg {
            Message::Tool(t) => {
                pending.push(ToolResultMessage {
                    tool_call_id: t.tool_call_id.clone(),
                    content: msg.text(),
                });
            }
            Message::System(_) => {
                flush(&mut out, &mut pending);
                out.push(ConversationMessage::Chat(ChatMessage::system(msg.text())));
            }
            Message::User(_) => {
                flush(&mut out, &mut pending);
                out.push(ConversationMessage::Chat(ChatMessage::user(msg.text())));
            }
            Message::Assistant(a) => {
                flush(&mut out, &mut pending);
                if a.tool_calls.is_empty() {
                    let mut chat = ChatMessage::assistant(msg.text());
                    chat.extra_metadata = reasoning_extra_metadata(&a.content);
                    out.push(ConversationMessage::Chat(chat));
                } else {
                    let text = msg.text();
                    out.push(ConversationMessage::AssistantToolCalls {
                        text: (!text.is_empty()).then_some(text),
                        tool_calls: a.tool_calls.iter().map(ta_call_to_oh_call).collect(),
                        reasoning_content: reasoning_from_content(&a.content),
                        extra_metadata: reasoning_extra_metadata(&a.content),
                    });
                }
            }
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// The suffix of `messages` produced *after* the most recent user turn — i.e.
/// the assistant/tool messages a single turn appended. Robust to front-trimming
/// middleware (which drops old messages but keeps the current user turn).
pub(super) fn messages_since_last_user(messages: &[Message]) -> &[Message] {
    let start = messages
        .iter()
        .rposition(|m| matches!(m, Message::User(_)))
        .map(|i| i + 1)
        .unwrap_or(0);
    &messages[start..]
}

/// Convert a harness transcript into openhuman [`ChatMessage`]s for a provider
/// that does **not** support native tool calls (text/prompt-guided mode).
///
/// Consecutive `Tool` result messages are coalesced into a single
/// `[Tool results]` user turn — the shape prompt-guided models are taught to
/// read — instead of native `tool`-role messages they wouldn't understand.
/// Other messages convert as usual (assistant tool calls already rode the
/// visible text in this mode).
pub(super) fn messages_to_text_mode_chat(messages: &[Message]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::new();
    let mut pending: Vec<String> = Vec::new();

    fn flush(out: &mut Vec<ChatMessage>, pending: &mut Vec<String>) {
        if !pending.is_empty() {
            out.push(ChatMessage::user(format!(
                "[Tool results]\n{}",
                std::mem::take(pending).join("\n")
            )));
        }
    }

    for msg in messages {
        match msg {
            Message::Tool(_) => pending.push(msg.text()),
            _ => {
                flush(&mut out, &mut pending);
                out.push(message_to_chat_message(msg));
            }
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// Convert an openhuman [`ToolSpec`] into a harness [`ToolSchema`].
pub(super) fn spec_to_schema(spec: &ToolSpec) -> ToolSchema {
    // `ToolSchema::new` sets the model-visible tool-call format to the JSON
    // default (tinyagents 1.0), which is what openhuman advertises.
    ToolSchema::new(
        spec.name.clone(),
        spec.description.clone(),
        spec.parameters.clone(),
    )
}

/// Convert a harness [`TaToolCall`] into an openhuman [`ToolCall`].
///
/// The harness models arguments as parsed JSON; openhuman carries them as the
/// raw JSON string the provider emitted, so we re-serialize.
pub(super) fn ta_call_to_oh_call(
    call: &TaToolCall,
) -> crate::openhuman::inference::provider::ToolCall {
    crate::openhuman::inference::provider::ToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.to_string(),
        extra_content: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_native_tool_round_recovers_structure_and_round_trips() {
        use crate::openhuman::inference::provider::ToolCall as OhToolCall;
        // The native dispatcher seeds an assistant tool round as a
        // {content, tool_calls} envelope followed by {tool_call_id, content} rows.
        let oh_call = OhToolCall {
            id: "call-1".into(),
            name: "echo".into(),
            arguments: r#"{"msg":"hi"}"#.into(),
            extra_content: None,
        };
        let assistant_cm = ChatMessage::assistant(
            serde_json::json!({ "content": "calling echo", "tool_calls": [oh_call] }).to_string(),
        );
        let tool_cm = ChatMessage::tool(
            serde_json::json!({ "tool_call_id": "call-1", "content": "echoed:hi" }).to_string(),
        );

        // Inbound: the envelopes are recovered into structured harness messages.
        let a = chat_message_to_message(&assistant_cm);
        let Message::Assistant(am) = &a else {
            panic!("expected Assistant, got {a:?}");
        };
        assert_eq!(am.tool_calls.len(), 1);
        assert_eq!(am.tool_calls[0].id, "call-1");
        assert_eq!(am.tool_calls[0].name, "echo");
        assert_eq!(
            am.tool_calls[0].arguments,
            serde_json::json!({ "msg": "hi" })
        );
        assert_eq!(a.text(), "calling echo");

        let t = chat_message_to_message(&tool_cm);
        let Message::Tool(tm) = &t else {
            panic!("expected Tool, got {t:?}");
        };
        assert_eq!(tm.tool_call_id, "call-1");
        assert_eq!(t.text(), "echoed:hi");

        // Outbound: re-serialized to a well-formed native tool round (assistant
        // carries structured tool_calls, the tool row carries the matching id).
        let a_native = message_to_native_chat_message(&a);
        assert_eq!(a_native.role, "assistant");
        let av: serde_json::Value = serde_json::from_str(&a_native.content).unwrap();
        assert_eq!(av["tool_calls"][0]["id"], "call-1");
        assert_eq!(av["content"], "calling echo");

        let t_native = message_to_native_chat_message(&t);
        assert_eq!(t_native.role, "tool");
        let tv: serde_json::Value = serde_json::from_str(&t_native.content).unwrap();
        assert_eq!(tv["tool_call_id"], "call-1");
        assert_eq!(tv["content"], "echoed:hi");
    }

    #[test]
    fn plain_assistant_prose_is_not_misread_as_a_tool_round() {
        let a = chat_message_to_message(&ChatMessage::assistant("just a normal reply"));
        let Message::Assistant(am) = &a else {
            panic!("expected Assistant, got {a:?}");
        };
        assert!(am.tool_calls.is_empty());
        assert_eq!(a.text(), "just a normal reply");
    }

    #[test]
    fn roles_round_trip_through_the_bridge() {
        let history = vec![
            ChatMessage::system("you are helpful"),
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi there"),
        ];
        let messages = history_to_messages(&history);
        assert!(matches!(messages[0], Message::System(_)));
        assert!(matches!(messages[1], Message::User(_)));
        assert!(matches!(messages[2], Message::Assistant(_)));

        let back = messages_to_history(&messages);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].role, "system");
        assert_eq!(back[1].content, "hello");
        assert_eq!(back[2].role, "assistant");
    }

    #[test]
    fn tool_message_preserves_correlation_id() {
        let messages = vec![Message::Tool(ToolMessage {
            tool_call_id: "call-7".into(),
            content: vec![ContentBlock::Text("done".into())],
        })];
        let back = messages_to_history(&messages);
        assert_eq!(back[0].role, "tool");
        assert_eq!(back[0].content, "done");
        assert_eq!(back[0].id.as_deref(), Some("call-7"));
    }

    #[test]
    fn conversation_preserves_tool_call_structure() {
        let messages = vec![
            Message::User(UserMessage {
                content: vec![ContentBlock::Text("do it".into())],
            }),
            Message::Assistant(AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text("calling".into())],
                tool_calls: vec![TaToolCall {
                    id: "c1".into(),
                    name: "echo".into(),
                    arguments: serde_json::json!({"msg": "hi"}),
                }],
                usage: None,
            }),
            Message::Tool(ToolMessage {
                tool_call_id: "c1".into(),
                content: vec![ContentBlock::Text("echoed:hi".into())],
            }),
            Message::Assistant(AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text("all done".into())],
                tool_calls: vec![],
                usage: None,
            }),
        ];

        // Only the suffix after the last user turn is persisted.
        let suffix = messages_since_last_user(&messages);
        let convo = messages_to_conversation(suffix);
        assert_eq!(convo.len(), 3);
        match &convo[0] {
            ConversationMessage::AssistantToolCalls { tool_calls, .. } => {
                assert_eq!(tool_calls[0].name, "echo");
                assert_eq!(tool_calls[0].id, "c1");
            }
            other => panic!("expected AssistantToolCalls, got {other:?}"),
        }
        match &convo[1] {
            ConversationMessage::ToolResults(results) => {
                assert_eq!(results[0].tool_call_id, "c1");
                assert_eq!(results[0].content, "echoed:hi");
            }
            other => panic!("expected ToolResults, got {other:?}"),
        }
        match &convo[2] {
            ConversationMessage::Chat(c) => {
                assert_eq!(c.role, "assistant");
                assert_eq!(c.content, "all done");
            }
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn spec_and_tool_call_convert() {
        let spec = ToolSpec {
            name: "echo".into(),
            description: "echoes".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let schema = spec_to_schema(&spec);
        assert_eq!(schema.name, "echo");
        assert_eq!(schema.parameters, serde_json::json!({"type": "object"}));

        let ta = TaToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({"msg": "hi"}),
        };
        let oh = ta_call_to_oh_call(&ta);
        assert_eq!(oh.id, "c1");
        assert_eq!(oh.name, "echo");
        assert_eq!(oh.arguments, r#"{"msg":"hi"}"#);
    }
}
