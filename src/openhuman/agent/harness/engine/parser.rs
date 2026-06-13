//! Response-parsing seam.
//!
//! The channel loop and subagent extract tool calls from a provider response
//! with the built-in native-first + XML-fallback logic ([`DefaultParser`]).
//! `Agent::turn` instead uses its configured [`ToolDispatcher`] (native / XML /
//! PFormat) — PFormat in particular parses positional `name[args]` calls the
//! built-in path can't. [`DispatcherParser`] adapts a dispatcher to this seam so
//! the engine stays parser-agnostic while preserving every dispatcher's grammar.
//!
//! `parse` returns `(display_text, calls)`: the narrative text to surface (tool
//! markup stripped) and the parsed calls in the engine's internal
//! [`ParsedToolCall`] shape. The engine keeps the *raw* response text
//! separately for assistant-history serialization.

use crate::openhuman::agent::dispatcher::ToolDispatcher;
use crate::openhuman::agent::harness::parse::{
    parse_structured_tool_calls, parse_tool_calls, ParsedToolCall,
};
use crate::openhuman::inference::provider::ChatResponse;

pub(crate) trait ResponseParser: Send + Sync {
    /// Returns `(display_text, calls)` for this provider response.
    fn parse(&self, resp: &ChatResponse) -> (String, Vec<ParsedToolCall>);
}

/// Built-in parser: prefer native structured tool calls, fall back to the
/// XML-tag parser over the response text. Used by the channel loop + subagent.
pub(crate) struct DefaultParser;

impl ResponseParser for DefaultParser {
    fn parse(&self, resp: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        let response_text = resp.text_or_empty().to_string();
        let mut calls = parse_structured_tool_calls(&resp.tool_calls);
        let mut parsed_text = String::new();
        if calls.is_empty() {
            let (fallback_text, fallback_calls) = parse_tool_calls(&response_text);
            if !fallback_text.is_empty() {
                parsed_text = fallback_text;
            }
            calls = fallback_calls;
        }
        let display_text = if parsed_text.is_empty() {
            response_text
        } else {
            parsed_text
        };
        (display_text, calls)
    }
}

/// Adapts an [`Agent`]'s configured [`ToolDispatcher`] to the parser seam,
/// converting the dispatcher's `ParsedToolCall` shape into the engine's.
pub(crate) struct DispatcherParser<'a> {
    pub dispatcher: &'a dyn ToolDispatcher,
}

impl ResponseParser for DispatcherParser<'_> {
    fn parse(&self, resp: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        let (text, calls) = self.dispatcher.parse_response(resp);
        let calls = calls
            .into_iter()
            .map(|c| ParsedToolCall {
                name: c.name,
                arguments: c.arguments,
                id: c.tool_call_id,
            })
            .collect();
        (text, calls)
    }
}
