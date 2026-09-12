//! Translate `ClaudeCodeEvent`s into OpenHuman `ProviderDelta`s plus a
//! final aggregated `ChatResponse`.
//!
//! The CLI emits content as anthropic-style content blocks. We map:
//!   - `content_block_start` text  → start a text accumulator
//!   - `content_block_delta` text  → `ProviderDelta::TextDelta`
//!   - `content_block_*`      tool → tracked but **NOT surfaced** (see below)
//!   - `result`                    → finalize usage + cost
//!
//! Thinking blocks (`thinking_delta`) are forwarded as
//! `ProviderDelta::ThinkingDelta`.
//!
//! ## Tool blocks are deliberately not surfaced
//!
//! The `claude` CLI is a **self-executing** agent: it runs its own built-in
//! tools (`Read`/`Write`/`Edit`/`Bash`/`Glob`/…) and OpenHuman's MCP tools
//! internally, in its own agent loop, and returns their results itself. So a
//! `tool_use` block in the stream is a call the CLI has **already executed** —
//! not a request for OpenHuman to run something.
//!
//! If we surfaced these as OpenHuman [`ToolCall`]s, the tinyagents harness would
//! try to dispatch them, hold none of them by those names, and reject each one
//! (`unknown tool \`Read\`; valid tools: [...]`) — injecting that corrective
//! back into the transcript and looping until it aborts (`N tool calls in a row
//! failed with no progress`). That is the intermittent "something went wrong"
//! wall users hit (it only bit when a `tool_use` block arrived as a streamed
//! partial rather than solely in the skipped final `assistant` message).
//!
//! So we track a `tool_use` block only enough to keep its `input_json_delta`s
//! out of the visible text, and surface nothing. The CLI's final assistant text
//! is the turn's result; its live narration/thinking still streams to the UI.

use std::collections::HashMap;

use serde_json::Value;

use super::stream_parser::ClaudeCodeEvent;
use crate::openhuman::inference::provider::ops::sanitize_api_error;
use crate::openhuman::inference::provider::types::{
    ChatResponse, ProviderDelta, ToolCall, UsageInfo,
};

fn result_diagnostic(raw: &Value) -> Option<String> {
    let values = raw
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(raw.get("result"))
        .chain(raw.get("error"))
        .chain(raw.get("message"));

    let messages: Vec<String> = values
        .filter_map(|value| {
            value.as_str().map(str::to_string).or_else(|| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
        .collect();

    (!messages.is_empty()).then(|| sanitize_api_error(&messages.join("; ")))
}

#[derive(Debug, Clone)]
struct BlockState {
    kind: BlockKind,
    tool_name: Option<String>,
    text_accum: String,
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
    /// Always empty for the self-executing `claude` CLI: its `tool_use` blocks
    /// are calls it already ran itself, so they are never surfaced as OpenHuman
    /// tool calls (see the module docs). Kept as the response's `tool_calls`
    /// slot so the harness always sees a terminal, tool-less response.
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<UsageInfo>,
    pub error: Option<String>,
    /// Whether the terminal result reported a semantic failure. Kept separate
    /// from `error` so a turn that failed without saying why can still fall
    /// through to stderr, rather than reporting a synthetic string that reads
    /// like a diagnosis while carrying nothing.
    pub terminal_error: bool,
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
                self.terminal_error = true;
                if !message.trim().is_empty() {
                    self.error = Some(sanitize_api_error(&message));
                }
                Vec::new()
            }
            ClaudeCodeEvent::Result {
                subtype,
                is_error,
                usage,
                total_cost_usd,
                raw,
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
                if is_error || subtype.as_deref() == Some("error") {
                    self.terminal_error = true;
                    if self.error.is_none() {
                        self.error = result_diagnostic(&raw);
                    }
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
                        tool_name: None,
                        text_accum: String::new(),
                    },
                );
                Vec::new()
            }
            "thinking" => {
                self.blocks.insert(
                    index,
                    BlockState {
                        kind: BlockKind::Thinking,
                        tool_name: None,
                        text_accum: String::new(),
                    },
                );
                Vec::new()
            }
            "tool_use" => {
                // The CLI has ALREADY executed this tool itself (see the module
                // docs). Track the block so its argument deltas don't leak into
                // the visible text, but surface no `ToolCallStart` — OpenHuman's
                // harness must never try to re-dispatch a call the CLI ran.
                let tool_name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                if let Some(name) = tool_name.as_deref() {
                    log::debug!(
                        "[claude-code][event-mapper] CLI self-executed tool `{name}` (not surfaced to the harness)"
                    );
                }
                // The block is tracked so its `input_json_delta`s and its stop
                // event are swallowed rather than leaking, but it is NOT
                // surfaced to OpenHuman's harness — see the note on
                // `on_block_stop`.
                self.blocks.insert(
                    index,
                    BlockState {
                        kind: BlockKind::Tool,
                        tool_name,
                        text_accum: String::new(),
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
                // Self-executed by the CLI (see the module docs). Discard the
                // argument fragments: they are not surfaced to the harness and
                // retaining them until block stop only wastes memory.
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
                "[claude-code][event-mapper] dropping cli-internal tool call name={}",
                state.tool_name.unwrap_or_default(),
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
