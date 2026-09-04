//! Translate `ClaudeCodeEvent`s into OpenHuman `ProviderDelta`s plus a
//! final aggregated `ChatResponse`.
//!
//! The CLI emits content as anthropic-style content blocks. We map:
//!   - `content_block_start` text  → start a text accumulator
//!   - `content_block_delta` text  → `ProviderDelta::TextDelta`
//!   - `content_block_start` tool  → `ProviderDelta::ToolCallStart`
//!   - `content_block_delta` tool  → `ProviderDelta::ToolCallArgsDelta`
//!   - `result`                    → finalize usage + cost
//!
//! Thinking blocks (`thinking_delta`) are forwarded as
//! `ProviderDelta::ThinkingDelta`.

use std::collections::HashMap;

use serde_json::Value;

use super::stream_parser::ClaudeCodeEvent;
use crate::openhuman::inference::provider::types::{
    ChatResponse, ProviderDelta, ToolCall, UsageInfo,
};

#[derive(Debug, Clone)]
struct BlockState {
    kind: BlockKind,
    call_id: Option<String>,
    tool_name: Option<String>,
    text_accum: String,
    input_accum: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    Tool,
}

#[derive(Debug, Default)]
pub struct EventMapper {
    blocks: HashMap<u64, BlockState>,
    pub final_text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<UsageInfo>,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub finished: bool,
}

impl EventMapper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one event and return the deltas to forward to the stream
    /// sink (if any).
    pub fn handle(&mut self, event: ClaudeCodeEvent) -> Vec<ProviderDelta> {
        match event {
            ClaudeCodeEvent::System { session_id, .. } => {
                if let Some(id) = session_id {
                    self.session_id = Some(id);
                }
                Vec::new()
            }
            ClaudeCodeEvent::Error { message } => {
                self.error = Some(message);
                Vec::new()
            }
            ClaudeCodeEvent::Result {
                subtype,
                usage,
                total_cost_usd,
                ..
            } => {
                let mut parsed = usage.as_ref().map(parse_usage);
                // CC stream emits `total_cost_usd` on the terminal `result`
                // event — surface it as `UsageInfo.charged_amount_usd` so
                // downstream cost.rs can record it without re-pricing
                // tokens × model rates.
                if let Some(cost) = total_cost_usd {
                    let usage = parsed.get_or_insert_with(UsageInfo::default);
                    usage.charged_amount_usd = cost;
                }
                self.usage = parsed;
                if subtype.as_deref() == Some("error") && self.error.is_none() {
                    self.error = Some("claude reported `result.subtype=error`".into());
                }
                self.finished = true;
                Vec::new()
            }
            ClaudeCodeEvent::Assistant { message } => {
                // CC 2.x emits a final assembled `assistant` event with
                // `message.type == "message"` after streaming completes via
                // `stream_event`. Skip to avoid double-emission.
                if message.get("type").and_then(Value::as_str) == Some("message") {
                    return Vec::new();
                }
                self.handle_assistant_block(&message)
            }
            ClaudeCodeEvent::StreamEvent { event } => self.handle_assistant_block(&event),
            ClaudeCodeEvent::User { message } => {
                // tool_result blocks from the CLI's own tool runs aren't
                // surfaced to OpenHuman's harness (the harness owns tools
                // via MCP, not via CC internals). Track for completeness.
                let _ = message;
                Vec::new()
            }
            ClaudeCodeEvent::RateLimit { .. } | ClaudeCodeEvent::ParseError { .. } => Vec::new(),
        }
    }

    fn handle_assistant_block(&mut self, msg: &Value) -> Vec<ProviderDelta> {
        let ty = msg.get("type").and_then(Value::as_str).unwrap_or("");
        let index = msg.get("index").and_then(Value::as_u64).unwrap_or(0);
        match ty {
            "content_block_start" => self.on_block_start(index, msg),
            "content_block_delta" => self.on_block_delta(index, msg),
            "content_block_stop" => self.on_block_stop(index),
            _ => Vec::new(),
        }
    }

    fn on_block_start(&mut self, index: u64, msg: &Value) -> Vec<ProviderDelta> {
        let block = match msg.get("content_block") {
            Some(b) => b,
            None => return Vec::new(),
        };
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "text" => {
                self.blocks.insert(
                    index,
                    BlockState {
                        kind: BlockKind::Text,
                        call_id: None,
                        tool_name: None,
                        text_accum: String::new(),
                        input_accum: String::new(),
                    },
                );
                Vec::new()
            }
            "thinking" => {
                self.blocks.insert(
                    index,
                    BlockState {
                        kind: BlockKind::Thinking,
                        call_id: None,
                        tool_name: None,
                        text_accum: String::new(),
                        input_accum: String::new(),
                    },
                );
                Vec::new()
            }
            "tool_use" => {
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let tool_name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                if call_id.is_none() || tool_name.is_none() {
                    log::warn!(
                        "[claude-code][event-mapper] skipping tool_use block with missing id or name"
                    );
                    return Vec::new();
                }
                let call_id = call_id.unwrap();
                let tool_name = tool_name.unwrap();
                // The block is tracked so its `input_json_delta`s and its stop
                // event are swallowed rather than leaking, but it is NOT
                // surfaced to OpenHuman's harness — see the note on
                // `on_block_stop`.
                log::debug!(
                    "[claude-code][event-mapper] cli-internal tool_use name={tool_name} id={call_id} (not surfaced)"
                );
                self.blocks.insert(
                    index,
                    BlockState {
                        kind: BlockKind::Tool,
                        call_id: Some(call_id),
                        tool_name: Some(tool_name),
                        text_accum: String::new(),
                        input_accum: String::new(),
                    },
                );
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_block_delta(&mut self, index: u64, msg: &Value) -> Vec<ProviderDelta> {
        let delta = match msg.get("delta") {
            Some(d) => d,
            None => return Vec::new(),
        };
        let dtype = delta.get("type").and_then(Value::as_str).unwrap_or("");
        let Some(state) = self.blocks.get_mut(&index) else {
            return Vec::new();
        };
        match (state.kind.clone(), dtype) {
            (BlockKind::Text, "text_delta") => {
                let text = delta
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                state.text_accum.push_str(&text);
                self.final_text.push_str(&text);
                vec![ProviderDelta::TextDelta { delta: text }]
            }
            (BlockKind::Thinking, "thinking_delta") => {
                let text = delta
                    .get("thinking")
                    .and_then(Value::as_str)
                    .or_else(|| delta.get("text").and_then(Value::as_str))
                    .unwrap_or("")
                    .to_string();
                state.text_accum.push_str(&text);
                vec![ProviderDelta::ThinkingDelta { delta: text }]
            }
            (BlockKind::Tool, "input_json_delta") => {
                let partial = delta
                    .get("partial_json")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // Accumulated for the debug log only; the call itself is the
                // CLI's to run, so nothing is emitted (see `on_block_stop`).
                state.input_accum.push_str(&partial);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_block_stop(&mut self, index: u64) -> Vec<ProviderDelta> {
        let Some(state) = self.blocks.remove(&index) else {
            return Vec::new();
        };
        if state.kind == BlockKind::Tool {
            // A native `tool_use` block from this CLI is the CLI's OWN call —
            // its builtins (Bash / Read / Write / Edit …) or a server from the
            // `--mcp-config` we hand it. The CLI executes them itself inside
            // its own agentic loop, which is why the matching `tool_result`
            // blocks are deliberately dropped in `map_event`.
            //
            // Surfacing the *call* while dropping its *result* handed
            // OpenHuman's harness a tool it does not own and cannot run: with
            // `full_access` on (no `--disallowedTools`), a turn that reached
            // for `Bash` produced repeated tool failures until the circuit
            // breaker halted the run, and the turn then burned its 900s
            // wall-clock backstop. So neither half is surfaced, and this
            // provider behaves as what it is — a chat model whose tool use is
            // internal. OpenHuman's own tools reach it through the prompt
            // catalogue, not through native tool calls.
            log::debug!(
                "[claude-code][event-mapper] dropping cli-internal tool call name={} args_len={}",
                state.tool_name.unwrap_or_default(),
                state.input_accum.len()
            );
        }
        Vec::new()
    }

    /// Build the final aggregated `ChatResponse` once the stream is done.
    pub fn into_response(self) -> ChatResponse {
        ChatResponse {
            text: if self.final_text.is_empty() {
                None
            } else {
                Some(self.final_text)
            },
            tool_calls: self.tool_calls,
            usage: self.usage,
            reasoning_content: None,
        }
    }
}

fn parse_usage(v: &Value) -> UsageInfo {
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    UsageInfo {
        input_tokens: n("input_tokens"),
        output_tokens: n("output_tokens"),
        context_window: 0,
        cached_input_tokens: n("cache_read_input_tokens"),
        cache_creation_tokens: n("cache_creation_input_tokens"),
        reasoning_tokens: 0,
        charged_amount_usd: 0.0,
    }
}

#[cfg(test)]
#[path = "event_mapper_tests.rs"]
mod tests;
