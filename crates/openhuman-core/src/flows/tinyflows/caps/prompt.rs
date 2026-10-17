//! Turning a node's request into a completion, and a completion back into a
//! node result.
//!
//! The pure request/reply handling (the `input_context` carrier, structured
//! output steering, tolerant JSON extraction) lives in
//! `tinyflows::nodes::integration::agent_prompt`; this file keeps the host
//! mapping of its neutral messages onto [`TranscriptMessage`], usage projection and
//! model pinning.

#![allow(unused_imports)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::config::Config;
use crate::inference::provider::{is_raw_passthrough_model, BilledUsage};
use tinyagents_session::transcript::TranscriptMessage;
use tinyflows::nodes::integration::agent_prompt;

/// [`agent_prompt::build_completion_messages`] mapped onto the host's
/// [`TranscriptMessage`] type.
pub(crate) fn build_completion_messages(request: &Value) -> Vec<TranscriptMessage> {
    agent_prompt::build_completion_messages(request)
        .into_iter()
        .map(|m| match m.role {
            "system" => TranscriptMessage::system(m.content),
            "assistant" => TranscriptMessage::assistant(m.content),
            "tool" => TranscriptMessage::tool(m.content),
            _ => TranscriptMessage::user(m.content),
        })
        .collect()
}

/// Maps a `BilledUsage` (not `Serialize`) into a JSON value field-by-field, so
/// [`OpenHumanLlm::complete`] can surface it in its response `Value` without
/// requiring an upstream `Serialize` impl change.
pub(crate) fn usage_to_json(usage: &Option<BilledUsage>) -> Value {
    match usage {
        None => Value::Null,
        Some(u) => json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "context_window": u.context_window(),
            "cached_input_tokens": u.cached_input_tokens(),
            "cache_creation_tokens": u.cache_creation_tokens,
            "reasoning_tokens": u.reasoning_tokens,
            "charged_amount_usd": u.charged_amount_usd,
        }),
    }
}

pub(crate) fn model_response_to_completion_value(
    response: &tinyinference_llm::model::ModelResponse,
) -> Value {
    json!({
        "text": response.text(),
        "tool_calls": response
            .tool_calls()
            .iter()
            .map(crate::agent::tinyagents::ta_call_to_oh_call)
            .collect::<Vec<_>>(),
        "usage": usage_to_json(
            &crate::agent::tinyagents::model::usage_info_from_response(response)
        ),
        "reasoning_content": tinyagents_runtime::transcript::reasoning_from_content(
            &response.message.content
        ),
    })
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
