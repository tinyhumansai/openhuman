use crate::openhuman::agent::harness::parse_tool_calls;
use crate::openhuman::agent::pformat::{self, PFormatRegistry};
use crate::openhuman::context::prompt::ToolCallFormat;
use crate::openhuman::providers::{
    ChatMessage, ChatResponse, ConversationMessage, ToolResultMessage,
};
use crate::openhuman::tools::{Tool, ToolSpec};
use serde_json::Value;
use std::fmt::Write;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: Value,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub name: String,
    pub output: String,
    pub success: bool,
    pub tool_call_id: Option<String>,
}

pub trait ToolDispatcher: Send + Sync {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>);
    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage;
    fn prompt_instructions(&self, tools: &[Box<dyn Tool>]) -> String;
    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage>;
    fn should_send_tool_specs(&self) -> bool;

    /// Tell the prompt builder how to render each tool entry in the
    /// `## Tools` section. Defaults to [`ToolCallFormat::Json`] for
    /// dispatchers that haven't opted in — `ToolsSection` then uses
    /// the historic schema-dump rendering.
    ///
    /// `PFormatToolDispatcher` overrides this to return
    /// [`ToolCallFormat::PFormat`] so the catalogue shows positional
    /// signatures (`get_weather[location|unit]`) instead of full JSON
    /// schemas — that's where most of the token saving comes from at
    /// the prompt level.
    fn tool_call_format(&self) -> ToolCallFormat {
        ToolCallFormat::Json
    }
}

#[derive(Default)]
pub struct XmlToolDispatcher;

impl XmlToolDispatcher {
    fn parse_tool_calls_from_text(response: &str) -> (String, Vec<ParsedToolCall>) {
        let (text, calls) = parse_tool_calls(response);
        let parsed_calls = calls
            .into_iter()
            .map(|call| ParsedToolCall {
                name: call.name,
                arguments: call.arguments,
                tool_call_id: None,
            })
            .collect::<Vec<_>>();
        (text, parsed_calls)
    }

    pub fn tool_specs(tools: &[Box<dyn Tool>]) -> Vec<ToolSpec> {
        tools.iter().map(|tool| tool.spec()).collect()
    }
}

impl ToolDispatcher for XmlToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        let text = response.text_or_empty();
        let (parsed_text, parsed_calls) = Self::parse_tool_calls_from_text(text);
        tracing::debug!(
            parse_mode = "text_fallback",
            parsed_tool_calls = parsed_calls.len(),
            "xml dispatcher parsed response"
        );
        (parsed_text, parsed_calls)
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        let mut content = String::new();
        for result in results {
            let status = if result.success { "ok" } else { "error" };
            let _ = writeln!(
                content,
                "<tool_result name=\"{}\" status=\"{}\">\n{}\n</tool_result>",
                result.name, status, result.output
            );
        }
        ConversationMessage::Chat(ChatMessage::user(format!("[Tool results]\n{content}")))
    }

    fn prompt_instructions(&self, tools: &[Box<dyn Tool>]) -> String {
        let mut instructions = String::new();
        instructions.push_str("## Tool Use Protocol\n\n");
        instructions
            .push_str("To use a tool, wrap a JSON object in <tool_call></tool_call> tags:\n\n");
        instructions.push_str(
            "```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n",
        );
        instructions.push_str("### Available Tools\n\n");

        for tool in tools {
            let _ = writeln!(
                instructions,
                "- **{}**: {}\n  Parameters: `{}`",
                tool.name(),
                tool.description(),
                tool.parameters_schema()
            );
        }

        instructions
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        history
            .iter()
            .flat_map(|msg| match msg {
                ConversationMessage::Chat(chat) => vec![chat.clone()],
                ConversationMessage::AssistantToolCalls { text, .. } => {
                    vec![ChatMessage::assistant(text.clone().unwrap_or_default())]
                }
                ConversationMessage::ToolResults(results) => {
                    let mut content = String::new();
                    for result in results {
                        let _ = writeln!(
                            content,
                            "<tool_result id=\"{}\">\n{}\n</tool_result>",
                            result.tool_call_id, result.content
                        );
                    }
                    vec![ChatMessage::user(format!("[Tool results]\n{content}"))]
                }
            })
            .collect()
    }

    fn should_send_tool_specs(&self) -> bool {
        false
    }
}

/// Text-based dispatcher that emits and parses **P-Format** ("Parameter
/// Format") tool calls — the compact `tool_name[arg1|arg2|...]` syntax
/// defined in [`crate::openhuman::agent::pformat`].
///
/// This is the default dispatcher for providers that do not support
/// native structured tool calls. Compared to the legacy
/// [`XmlToolDispatcher`] (XML wrapper + JSON body), p-format cuts the
/// per-call token cost by ~80% — a single weather lookup goes from
/// ~25 tokens to ~5 — which compounds dramatically over a long agent
/// loop.
///
/// The dispatcher caches a [`PFormatRegistry`] (a `name → params`
/// lookup) at construction time so it never has to hold a reference to
/// the live `Vec<Box<dyn Tool>>` (which the [`Agent`] owns). The
/// caller is expected to build the registry from the same tool slice
/// they pass into the agent — see `pformat::build_registry`.
///
/// On the parse side the dispatcher tries p-format **first** and falls
/// back to the existing JSON-in-tag parser if the body doesn't match
/// the bracket pattern. This keeps the dispatcher backwards-compatible
/// with models that still emit JSON tool calls — they just pay the
/// usual token cost for their bytes.
pub struct PFormatToolDispatcher {
    registry: Arc<PFormatRegistry>,
}

impl PFormatToolDispatcher {
    pub fn new(registry: PFormatRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
        }
    }

    /// Convert the registry-driven parser output into the dispatcher's
    /// `ParsedToolCall` shape. Always called inside a `<tool_call>` tag
    /// body — the tag-finding logic comes from the shared
    /// [`parse_tool_calls`] helper.
    fn try_parse_pformat_body(&self, body: &str) -> Option<ParsedToolCall> {
        let (name, args) = pformat::parse_call(body, self.registry.as_ref())?;
        Some(ParsedToolCall {
            name,
            arguments: args,
            tool_call_id: None,
        })
    }
}

impl ToolDispatcher for PFormatToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        let text = response.text_or_empty();

        // Run the JSON parser first — it gives us the narrative text
        // and a Vec of JSON-parsed calls. We then walk the tags
        // ourselves and resolve each one individually: if p-format
        // succeeds, use that; otherwise keep the JSON entry. This
        // per-tag selection means a response mixing p-format and JSON
        // tags is handled correctly instead of the old all-or-nothing.
        //
        // `XmlToolDispatcher::parse_tool_calls_from_text` is the
        // canonical adapter from the internal `harness::parse`
        // `ParsedToolCall` to the dispatcher's `ParsedToolCall`.
        let json_pass = XmlToolDispatcher::parse_tool_calls_from_text(text);

        // Walk tags manually, building a combined list that prefers
        // p-format but falls back to JSON per tag.
        let mut combined_calls = Vec::new();
        let mut json_idx: usize = 0; // index into json_pass.1
        let mut remaining = text;
        let tags: &[(&str, &str)] = &[
            ("<tool_call>", "</tool_call>"),
            ("<toolcall>", "</toolcall>"),
            ("<tool-call>", "</tool-call>"),
            ("<invoke>", "</invoke>"),
        ];
        while !remaining.is_empty() {
            let next = tags
                .iter()
                .filter_map(|(open, close)| remaining.find(open).map(|i| (i, *open, *close)))
                .min_by_key(|(i, _, _)| *i);

            let Some((open_idx, open_tag, close_tag)) = next else {
                break;
            };

            let after_open = &remaining[open_idx + open_tag.len()..];
            let Some(close_idx) = after_open.find(close_tag) else {
                break;
            };

            let body = &after_open[..close_idx];

            // Try p-format first; if that fails, take the
            // corresponding JSON entry (if one exists at this index).
            if let Some(parsed) = self.try_parse_pformat_body(body) {
                combined_calls.push(parsed);
                // Advance the JSON index too — both parsers walk the
                // same ordered set of tags, so they stay in lockstep.
                json_idx += 1;
            } else if let Some(json_call) = json_pass.1.get(json_idx) {
                combined_calls.push(json_call.clone());
                json_idx += 1;
            }

            remaining = &after_open[close_idx + close_tag.len()..];
        }

        if !combined_calls.is_empty() {
            tracing::debug!(
                parse_mode = "pformat_combined",
                parsed_tool_calls = combined_calls.len(),
                "pformat dispatcher parsed response (per-tag selection)"
            );
            return (json_pass.0, combined_calls);
        }

        // No tags found at all (or all tags failed both parsers) —
        // return the full JSON pass which also handles markdown
        // code-block and GLM fallbacks.
        tracing::debug!(
            parse_mode = "pformat_fallback_json",
            parsed_tool_calls = json_pass.1.len(),
            "pformat dispatcher fell back to JSON-in-tag path"
        );
        json_pass
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        // Same wrapping format as XML dispatcher — `<tool_result>` tags
        // are unaffected by the call-side syntax change.
        let mut content = String::new();
        for result in results {
            let status = if result.success { "ok" } else { "error" };
            let _ = writeln!(
                content,
                "<tool_result name=\"{}\" status=\"{}\">\n{}\n</tool_result>",
                result.name, status, result.output
            );
        }
        ConversationMessage::Chat(ChatMessage::user(format!("[Tool results]\n{content}")))
    }

    fn prompt_instructions(&self, _tools: &[Box<dyn Tool>]) -> String {
        // Protocol description ONLY — the tool catalogue is rendered by
        // the upstream `ToolsSection` (which now reads
        // `PromptContext::tool_call_format` and emits the same positional
        // signatures we'd otherwise duplicate here). Keeping this string
        // protocol-only avoids the wasteful "tools listed twice" pattern
        // the legacy `XmlToolDispatcher` carries forward, and means
        // adding a new tool only changes the prompt in one place.
        let mut instructions = String::new();
        instructions.push_str("## Tool Use Protocol\n\n");
        instructions.push_str(
            "Tool calls use **P-Format** (Parameter-Format): compact, positional, \
             pipe-delimited syntax wrapped in `<tool_call>` tags. ~80% cheaper on tokens \
             than JSON.\n\n",
        );
        instructions
            .push_str("```\n<tool_call>\nget_weather[London|metric]\n</tool_call>\n```\n\n");
        instructions.push_str(
            "**Rules:**\n\
             - Form: `name[arg1|arg2|...|argN]`. Arguments are positional and must match the \
               order shown in each tool's `Call as:` signature in the `## Tools` section above \
               (alphabetical by parameter name).\n\
             - Empty calls: `name[]` for zero-arg tools.\n\
             - Empty argument: `name[||value]` is three positional values, the first two empty.\n\
             - Escapes inside argument values: `\\|` → `|`, `\\]` → `]`, `\\\\` → `\\`.\n\
             - You may emit multiple `<tool_call>` blocks in a single response. Each tag holds \
               exactly one call.\n\
             - After tool execution, results appear in `<tool_result>` tags. Continue reasoning \
               with the results until you can give a final answer.\n\
             - If you genuinely need a complex nested argument that p-format can't express, \
               you may fall back to the JSON form: \
               `<tool_call>{\"name\":\"...\",\"arguments\":{...}}</tool_call>`. Prefer p-format \
               for everything else.\n\n",
        );

        instructions
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        // Identical to XML dispatcher — history serialization is
        // independent of the call-body format.
        history
            .iter()
            .flat_map(|msg| match msg {
                ConversationMessage::Chat(chat) => vec![chat.clone()],
                ConversationMessage::AssistantToolCalls { text, .. } => {
                    vec![ChatMessage::assistant(text.clone().unwrap_or_default())]
                }
                ConversationMessage::ToolResults(results) => {
                    let mut content = String::new();
                    for result in results {
                        let _ = writeln!(
                            content,
                            "<tool_result id=\"{}\">\n{}\n</tool_result>",
                            result.tool_call_id, result.content
                        );
                    }
                    vec![ChatMessage::user(format!("[Tool results]\n{content}"))]
                }
            })
            .collect()
    }

    fn should_send_tool_specs(&self) -> bool {
        // P-format is text-based — the model never receives a structured
        // tool spec, only the catalogue inside the system prompt.
        false
    }

    fn tool_call_format(&self) -> ToolCallFormat {
        ToolCallFormat::PFormat
    }
}

pub struct NativeToolDispatcher;

impl ToolDispatcher for NativeToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        let text = response.text.clone().unwrap_or_default();
        let calls: Vec<ParsedToolCall> = response
            .tool_calls
            .iter()
            .map(|tc| ParsedToolCall {
                name: tc.name.clone(),
                arguments: serde_json::from_str(&tc.arguments).unwrap_or_else(|e| {
                    tracing::warn!(
                        tool = %tc.name,
                        error = %e,
                        "Failed to parse native tool call arguments as JSON; defaulting to empty object"
                    );
                    Value::Object(serde_json::Map::new())
                }),
                tool_call_id: Some(tc.id.clone()),
            })
            .collect();

        if !calls.is_empty() {
            tracing::debug!(
                parse_mode = "native_structured",
                parsed_tool_calls = calls.len(),
                "native dispatcher parsed response"
            );
            return (text, calls);
        }

        if !text.is_empty() {
            let (fallback_text, fallback_calls) =
                XmlToolDispatcher::parse_tool_calls_from_text(&text);
            if !fallback_calls.is_empty() {
                let display_text = if fallback_text.is_empty() {
                    text
                } else {
                    fallback_text
                };
                tracing::debug!(
                    parse_mode = "text_fallback",
                    parsed_tool_calls = fallback_calls.len(),
                    "native dispatcher parsed response"
                );
                return (display_text, fallback_calls);
            }
        }

        tracing::debug!(
            parse_mode = "none",
            parsed_tool_calls = 0,
            "native dispatcher parsed response"
        );
        (text, calls)
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        let messages = results
            .iter()
            .map(|result| ToolResultMessage {
                tool_call_id: result
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                content: result.output.clone(),
            })
            .collect();
        ConversationMessage::ToolResults(messages)
    }

    fn prompt_instructions(&self, _tools: &[Box<dyn Tool>]) -> String {
        [
            "## Tool Use Protocol",
            "",
            "When a tool is needed, emit tool calls directly via the model's native tool-calling output.",
            "Do not only narrate intent (for example, avoid \"Let me check...\") without emitting the tool call.",
            "After tool results are provided, continue reasoning and then produce the final answer.",
            "",
        ]
        .join("\n")
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        history
            .iter()
            .flat_map(|msg| match msg {
                ConversationMessage::Chat(chat) => vec![chat.clone()],
                ConversationMessage::AssistantToolCalls { text, tool_calls } => {
                    let payload = serde_json::json!({
                        "content": text,
                        "tool_calls": tool_calls,
                    });
                    vec![ChatMessage::assistant(payload.to_string())]
                }
                ConversationMessage::ToolResults(results) => results
                    .iter()
                    .map(|result| {
                        ChatMessage::tool(
                            serde_json::json!({
                                "tool_call_id": result.tool_call_id,
                                "content": result.content,
                            })
                            .to_string(),
                        )
                    })
                    .collect(),
            })
            .collect()
    }

    fn should_send_tool_specs(&self) -> bool {
        true
    }

    fn tool_call_format(&self) -> ToolCallFormat {
        ToolCallFormat::Native
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::pformat::PFormatToolParams;

    #[test]
    fn xml_dispatcher_parses_tool_calls() {
        let response = ChatResponse {
            text: Some(
                "Checking\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
        };
        let dispatcher = XmlToolDispatcher;
        let (_, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn native_dispatcher_roundtrip() {
        let response = ChatResponse {
            text: Some("ok".into()),
            tool_calls: vec![crate::openhuman::providers::ToolCall {
                id: "tc1".into(),
                name: "file_read".into(),
                arguments: "{\"path\":\"a.txt\"}".into(),
            }],
            usage: None,
        };
        let dispatcher = NativeToolDispatcher;
        let (_, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_call_id.as_deref(), Some("tc1"));

        let msg = dispatcher.format_results(&[ToolExecutionResult {
            name: "file_read".into(),
            output: "hello".into(),
            success: true,
            tool_call_id: Some("tc1".into()),
        }]);
        match msg {
            ConversationMessage::ToolResults(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].tool_call_id, "tc1");
            }
            _ => panic!("expected tool results"),
        }
    }

    #[test]
    fn native_dispatcher_falls_back_to_xml_tool_calls() {
        let response = ChatResponse {
            text: Some(
                "Checking files...\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
        };
        let dispatcher = NativeToolDispatcher;
        let (text, calls) = dispatcher.parse_response(&response);
        assert_eq!(text, "Checking files...");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
        assert_eq!(calls[0].tool_call_id, None);
    }

    #[test]
    fn native_dispatcher_falls_back_to_invoke_tag() {
        let response = ChatResponse {
            text: Some(
                "Let me run this.\n<invoke>{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}}</invoke>".into(),
            ),
            tool_calls: vec![],
            usage: None,
        };
        let dispatcher = NativeToolDispatcher;
        let (text, calls) = dispatcher.parse_response(&response);
        assert_eq!(text, "Let me run this.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn xml_format_results_contains_tool_result_tags() {
        let dispatcher = XmlToolDispatcher;
        let msg = dispatcher.format_results(&[ToolExecutionResult {
            name: "shell".into(),
            output: "ok".into(),
            success: true,
            tool_call_id: None,
        }]);
        let rendered = match msg {
            ConversationMessage::Chat(chat) => chat.content,
            _ => String::new(),
        };
        assert!(rendered.contains("<tool_result"));
        assert!(rendered.contains("shell"));
    }

    fn pformat_registry_for(name: &str, props: serde_json::Value) -> PFormatRegistry {
        let schema = serde_json::json!({
            "type": "object",
            "properties": props
        });
        let mut reg = PFormatRegistry::new();
        reg.insert(name.to_string(), PFormatToolParams::from_schema(&schema));
        reg
    }

    #[test]
    fn pformat_dispatcher_parses_tool_call_tag() {
        // The model emits a p-format call inside a `<tool_call>` tag.
        // The dispatcher should pull it out, look up the tool's
        // parameter ordering, and produce named JSON args.
        let registry = pformat_registry_for(
            "get_weather",
            serde_json::json!({
                "location": { "type": "string" },
                "unit": { "type": "string" }
            }),
        );
        let dispatcher = PFormatToolDispatcher::new(registry);
        let response = ChatResponse {
            text: Some(
                "Let me check the weather.\n<tool_call>get_weather[London|metric]</tool_call>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
        };
        let (text, calls) = dispatcher.parse_response(&response);
        assert_eq!(text, "Let me check the weather.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(
            calls[0].arguments,
            serde_json::json!({"location": "London", "unit": "metric"})
        );
    }

    #[test]
    fn pformat_dispatcher_falls_back_to_json_in_tag() {
        // A model that ignored the p-format protocol and emitted a
        // JSON tool call should still be parsed correctly — the
        // dispatcher's whole point is to be a strict superset of the
        // legacy XML behaviour.
        let registry = pformat_registry_for(
            "shell",
            serde_json::json!({ "command": { "type": "string" } }),
        );
        let dispatcher = PFormatToolDispatcher::new(registry);
        let response = ChatResponse {
            text: Some(
                "Running it now.\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
        };
        let (text, calls) = dispatcher.parse_response(&response);
        assert_eq!(text, "Running it now.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
        assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
    }

    #[test]
    fn pformat_dispatcher_handles_multiple_tags() {
        let registry = pformat_registry_for(
            "shell",
            serde_json::json!({ "command": { "type": "string" } }),
        );
        let dispatcher = PFormatToolDispatcher::new(registry);
        let response = ChatResponse {
            text: Some(
                "Step 1.\n<tool_call>shell[ls]</tool_call>\nStep 2.\n<tool_call>shell[pwd]</tool_call>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
        };
        let (_text, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
        assert_eq!(calls[1].arguments, serde_json::json!({"command": "pwd"}));
    }

    #[test]
    fn pformat_dispatcher_reports_pformat_tool_call_format() {
        let dispatcher = PFormatToolDispatcher::new(PFormatRegistry::new());
        assert_eq!(dispatcher.tool_call_format(), ToolCallFormat::PFormat);
    }

    #[test]
    fn pformat_dispatcher_instructions_are_protocol_only() {
        // The dispatcher's prompt_instructions should NOT re-render
        // the tool catalogue — that's `ToolsSection`'s job. Otherwise
        // every tool gets emitted twice and the prompt double-pays.
        let dispatcher = PFormatToolDispatcher::new(PFormatRegistry::new());
        // Pass in a tool to make sure the dispatcher ignores it.
        struct DummyTool;
        #[async_trait::async_trait]
        impl Tool for DummyTool {
            fn name(&self) -> &str {
                "should_not_appear"
            }
            fn description(&self) -> &str {
                "this string must not show up in the dispatcher instructions"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn execute(
                &self,
                _args: serde_json::Value,
            ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
                Ok(crate::openhuman::tools::ToolResult::success("ok"))
            }
        }
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(DummyTool)];
        let instructions = dispatcher.prompt_instructions(&tools);
        assert!(instructions.contains("Tool Use Protocol"));
        assert!(
            !instructions.contains("should_not_appear"),
            "dispatcher instructions must not duplicate the tool catalogue, got:\n{instructions}"
        );
    }

    #[test]
    fn native_format_results_keeps_tool_call_id() {
        let dispatcher = NativeToolDispatcher;
        let msg = dispatcher.format_results(&[ToolExecutionResult {
            name: "shell".into(),
            output: "ok".into(),
            success: true,
            tool_call_id: Some("tc-1".into()),
        }]);

        match msg {
            ConversationMessage::ToolResults(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].tool_call_id, "tc-1");
            }
            _ => panic!("expected ToolResults variant"),
        }
    }
}
