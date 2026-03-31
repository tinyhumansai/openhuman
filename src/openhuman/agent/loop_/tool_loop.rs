use crate::openhuman::agent::multimodal;
use crate::openhuman::approval::{ApprovalManager, ApprovalRequest, ApprovalResponse};
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider, ProviderCapabilityError};
use crate::openhuman::tools::Tool;
use anyhow::Result;
use std::fmt::Write as _;
use std::io::Write as _;

use super::credentials::scrub_credentials;
use super::parse::{
    build_native_assistant_history, find_tool, parse_structured_tool_calls, parse_tool_calls,
};

/// Minimum characters per chunk when relaying LLM text to a streaming draft.
const STREAM_CHUNK_MIN_CHARS: usize = 80;

/// Default maximum agentic tool-use iterations per user message to prevent runaway loops.
/// Used as a safe fallback when `max_tool_iterations` is unset or configured as zero.
pub(crate) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
/// When `silent` is true, suppresses stdout (for channel use).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn agent_turn(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    max_tool_iterations: usize,
) -> Result<String> {
    run_tool_call_loop(
        provider,
        history,
        tools_registry,
        provider_name,
        model,
        temperature,
        silent,
        None,
        "channel",
        multimodal_config,
        max_tool_iterations,
        None,
    )
    .await
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool_call_loop(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    approval: Option<&ApprovalManager>,
    channel_name: &str,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    max_tool_iterations: usize,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<String> {
    let max_iterations = if max_tool_iterations == 0 {
        DEFAULT_MAX_TOOL_ITERATIONS
    } else {
        max_tool_iterations
    };

    let tool_specs: Vec<crate::openhuman::tools::ToolSpec> =
        tools_registry.iter().map(|tool| tool.spec()).collect();
    let use_native_tools = provider.supports_native_tools() && !tool_specs.is_empty();

    for _iteration in 0..max_iterations {
        let image_marker_count = multimodal::count_image_markers(history);
        if image_marker_count > 0 && !provider.supports_vision() {
            return Err(ProviderCapabilityError {
                provider: provider_name.to_string(),
                capability: "vision".to_string(),
                message: format!(
                    "received {image_marker_count} image marker(s), but this provider does not support vision input"
                ),
            }
            .into());
        }

        let prepared_messages =
            multimodal::prepare_messages_for_provider(history, multimodal_config).await?;

        // Unified path via Provider::chat so provider-specific native tool logic
        // (OpenAI/Anthropic/OpenRouter/compatible adapters) is honored.
        let request_tools = if use_native_tools {
            Some(tool_specs.as_slice())
        } else {
            None
        };

        let (response_text, parsed_text, tool_calls, assistant_history_content, native_tool_calls) =
            match provider
                .chat(
                    ChatRequest {
                        messages: &prepared_messages.messages,
                        tools: request_tools,
                    },
                    model,
                    temperature,
                )
                .await
            {
                Ok(resp) => {
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

                    // Preserve native tool call IDs in assistant history so role=tool
                    // follow-up messages can reference the exact call id.
                    let assistant_history_content = if resp.tool_calls.is_empty() {
                        response_text.clone()
                    } else {
                        build_native_assistant_history(&response_text, &resp.tool_calls)
                    };

                    let native_calls = resp.tool_calls;
                    (
                        response_text,
                        parsed_text,
                        calls,
                        assistant_history_content,
                        native_calls,
                    )
                }
                Err(e) => {
                    return Err(e);
                }
            };

        let display_text = if parsed_text.is_empty() {
            response_text.clone()
        } else {
            parsed_text
        };

        if tool_calls.is_empty() {
            // No tool calls — this is the final response.
            // If a streaming sender is provided, relay the text in small chunks
            // so the channel can progressively update the draft message.
            if let Some(ref tx) = on_delta {
                // Split on whitespace boundaries, accumulating chunks of at least
                // STREAM_CHUNK_MIN_CHARS characters for progressive draft updates.
                let mut chunk = String::new();
                for word in display_text.split_inclusive(char::is_whitespace) {
                    chunk.push_str(word);
                    if chunk.len() >= STREAM_CHUNK_MIN_CHARS
                        && tx.send(std::mem::take(&mut chunk)).await.is_err()
                    {
                        break; // receiver dropped
                    }
                }
                if !chunk.is_empty() {
                    let _ = tx.send(chunk).await;
                }
            }
            history.push(ChatMessage::assistant(response_text.clone()));
            return Ok(display_text);
        }

        // Print any text the LLM produced alongside tool calls (unless silent)
        if !silent && !display_text.is_empty() {
            print!("{display_text}");
            let _ = std::io::stdout().flush();
        }

        // Execute each tool call and build results.
        // `individual_results` tracks per-call output so that native-mode history
        // can emit one `role: tool` message per tool call with the correct ID.
        let mut tool_results = String::new();
        let mut individual_results: Vec<String> = Vec::new();
        for call in &tool_calls {
            // ── Approval hook ────────────────────────────────
            if let Some(mgr) = approval {
                if mgr.needs_approval(&call.name) {
                    let request = ApprovalRequest {
                        tool_name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    };

                    // Only prompt interactively when approvals are supported; auto-approve on other channels.
                    let decision = if channel_name == "cli" {
                        mgr.prompt_cli(&request)
                    } else {
                        ApprovalResponse::Yes
                    };

                    mgr.record_decision(&call.name, &call.arguments, decision, channel_name);

                    if decision == ApprovalResponse::No {
                        let denied = "Denied by user.".to_string();
                        individual_results.push(denied.clone());
                        let _ = writeln!(
                            tool_results,
                            "<tool_result name=\"{}\">\n{denied}\n</tool_result>",
                            call.name
                        );
                        continue;
                    }
                }
            }

            let result = if let Some(tool) = find_tool(tools_registry, &call.name) {
                match tool.execute(call.arguments.clone()).await {
                    Ok(r) => {
                        if r.success {
                            scrub_credentials(&r.output)
                        } else {
                            format!("Error: {}", r.error.unwrap_or(r.output))
                        }
                    }
                    Err(e) => {
                        format!("Error executing {}: {e}", call.name)
                    }
                }
            } else {
                format!("Unknown tool: {}", call.name)
            };

            individual_results.push(result.clone());
            let _ = writeln!(
                tool_results,
                "<tool_result name=\"{}\">\n{}\n</tool_result>",
                call.name, result
            );
        }

        // Add assistant message with tool calls + tool results to history.
        // Native mode: use JSON-structured messages so convert_messages() can
        // reconstruct proper OpenAI-format tool_calls and tool result messages.
        // Prompt mode: use XML-based text format as before.
        history.push(ChatMessage::assistant(assistant_history_content));
        if native_tool_calls.is_empty() {
            history.push(ChatMessage::user(format!("[Tool results]\n{tool_results}")));
        } else {
            for (native_call, result) in native_tool_calls.iter().zip(individual_results.iter()) {
                let tool_msg = serde_json::json!({
                    "tool_call_id": native_call.id,
                    "content": result,
                });
                history.push(ChatMessage::tool(tool_msg.to_string()));
            }
        }
    }

    anyhow::bail!("Agent exceeded maximum tool iterations ({max_iterations})")
}
