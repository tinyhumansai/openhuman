use crate::openhuman::agent::multimodal;
use crate::openhuman::approval::{ApprovalManager, ApprovalRequest, ApprovalResponse};
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider, ProviderCapabilityError};
use crate::openhuman::tools::traits::ToolScope;
use crate::openhuman::tools::Tool;
use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write as _;

use super::credentials::scrub_credentials;
use super::parse::{
    build_native_assistant_history, parse_structured_tool_calls, parse_tool_calls,
};
use crate::openhuman::context::guard::{ContextCheckResult, ContextGuard};

/// Minimum characters per chunk when relaying LLM text to a streaming draft.
const STREAM_CHUNK_MIN_CHARS: usize = 80;

/// Default maximum agentic tool-use iterations per user message to prevent runaway loops.
/// Used as a safe fallback when `max_tool_iterations` is unset or configured as zero.
pub(crate) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
/// When `silent` is true, suppresses stdout (for channel use).
///
/// This is a thin wrapper around [`run_tool_call_loop`] with the per-agent
/// filter and extra-tool plumbing disabled — i.e. the LLM sees the entire
/// `tools_registry` unchanged. Used by legacy call sites and harness tests
/// that don't need agent-aware scoping.
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
        None,
        &[],
    )
    .await
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
///
/// # Per-agent tool scoping
///
/// The last two parameters support per-agent tool filtering without
/// requiring callers to build a filtered copy of the (non-`Clone`able)
/// tool registry:
///
/// * `visible_tool_names` — optional whitelist of tool names that are
///   allowed to reach the LLM. When `Some(set)`, only tools whose
///   `name()` is present in the set contribute to the function-calling
///   schema and are eligible for execution; every other tool in the
///   registry is hidden from the model and rejected if the model
///   somehow emits a call for it. When `None`, no filtering is applied
///   and every tool in the combined registry is visible (the legacy
///   behaviour used by CLI/REPL and harness tests).
///
/// * `extra_tools` — per-turn synthesised tools to splice alongside the
///   persistent `tools_registry`. The agent-dispatch path uses this to
///   surface delegation tools (`research`, `delegate_gmail`, …) that
///   are synthesised fresh per turn from the active agent's
///   `subagents` field and the current Composio integration list, and
///   therefore are not registered in the global startup-time registry.
///
/// The combined tool list seen by the LLM this turn is
/// `tools_registry.iter().chain(extra_tools.iter())`, further narrowed
/// by `visible_tool_names` when supplied.
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
    visible_tool_names: Option<&HashSet<String>>,
    extra_tools: &[Box<dyn Tool>],
) -> Result<String> {
    let max_iterations = if max_tool_iterations == 0 {
        DEFAULT_MAX_TOOL_ITERATIONS
    } else {
        max_tool_iterations
    };

    // Is a given tool name visible to the model this turn? `None`
    // means no filter (legacy behaviour = everything visible).
    let is_visible = |name: &str| -> bool {
        match visible_tool_names {
            Some(set) => set.contains(name),
            None => true,
        }
    };

    let tool_specs: Vec<crate::openhuman::tools::ToolSpec> = tools_registry
        .iter()
        .chain(extra_tools.iter())
        .filter(|tool| is_visible(tool.name()))
        .map(|tool| tool.spec())
        .collect();
    let use_native_tools = provider.supports_native_tools() && !tool_specs.is_empty();

    log::debug!(
        "[tool-loop] Registry has {} tool(s), extra {} tool(s), filter={} — {} visible in schema: [{}]",
        tools_registry.len(),
        extra_tools.len(),
        visible_tool_names
            .map(|s| format!("whitelist({})", s.len()))
            .unwrap_or_else(|| "none".to_string()),
        tool_specs.len(),
        tool_specs
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut context_guard = ContextGuard::new();

    for iteration in 0..max_iterations {
        // ── Context guard: check utilization before each LLM call ──
        match context_guard.check() {
            ContextCheckResult::Ok => {}
            ContextCheckResult::CompactionNeeded => {
                tracing::warn!(
                    iteration,
                    "[agent_loop] context guard: compaction needed (>{:.0}% full)",
                    crate::openhuman::context::guard::COMPACTION_TRIGGER_THRESHOLD * 100.0
                );
                // Compaction is handled by history management upstream;
                // log and continue so the caller can act on it.
            }
            ContextCheckResult::ContextExhausted {
                utilization_pct,
                reason,
            } => {
                tracing::error!(
                    iteration,
                    utilization_pct,
                    "[agent_loop] context exhausted, aborting: {reason}"
                );
                anyhow::bail!("Context window exhausted ({utilization_pct}% full): {reason}");
            }
        }

        tracing::debug!(iteration, "[agent_loop] sending LLM request");
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
                        system_prompt_cache_boundary: None,
                    },
                    model,
                    temperature,
                )
                .await
            {
                Ok(resp) => {
                    // Update context guard with token usage from this response.
                    if let Some(ref usage) = resp.usage {
                        context_guard.update_usage(usage);
                        tracing::debug!(
                            iteration,
                            input_tokens = usage.input_tokens,
                            output_tokens = usage.output_tokens,
                            context_window = usage.context_window,
                            "[agent_loop] LLM response received"
                        );
                    } else {
                        tracing::debug!(
                            iteration,
                            "[agent_loop] LLM response received (no usage info)"
                        );
                    }

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

                    tracing::debug!(
                        iteration,
                        native_tool_calls = resp.tool_calls.len(),
                        parsed_tool_calls = calls.len(),
                        "[agent_loop] tool calls parsed"
                    );

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
            tracing::debug!(
                iteration,
                "[agent_loop] no tool calls — returning final response"
            );
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

            // Look up the tool by name in the combined registry + extras,
            // subject to the visibility whitelist. If the model hallucinated
            // a filtered-out tool name we treat it as unknown — the error
            // path below produces a structured error message the LLM can
            // correct in the next iteration.
            let tool_opt: Option<&dyn Tool> = tools_registry
                .iter()
                .chain(extra_tools.iter())
                .find(|t| t.name() == call.name && is_visible(t.name()))
                .map(|b| b.as_ref());
            tracing::debug!(
                iteration,
                tool = call.name.as_str(),
                found = tool_opt.is_some(),
                "[agent_loop] executing tool"
            );

            // Scope check: CliRpcOnly tools cannot run in the autonomous agent loop.
            if let Some(tool) = tool_opt {
                if tool.scope() == ToolScope::CliRpcOnly {
                    tracing::warn!(
                        iteration,
                        tool = call.name.as_str(),
                        "[agent_loop] tool scope is CliRpcOnly — denied in agent loop"
                    );
                    let denied = format!(
                        "Tool '{}' is only available via explicit CLI/RPC invocation, not in the autonomous agent loop.",
                        call.name
                    );
                    individual_results.push(denied.clone());
                    let _ = writeln!(
                        tool_results,
                        "<tool_result name=\"{}\">\n{denied}\n</tool_result>",
                        call.name
                    );
                    continue;
                }
            }

            let result = if let Some(tool) = tool_opt {
                let tool_deadline =
                    crate::openhuman::tool_timeout::tool_execution_timeout_duration();
                let timeout_secs = crate::openhuman::tool_timeout::tool_execution_timeout_secs();
                match tokio::time::timeout(tool_deadline, tool.execute(call.arguments.clone()))
                    .await
                {
                    Ok(Ok(r)) => {
                        let output = r.output();
                        if !r.is_error {
                            tracing::debug!(
                                iteration,
                                tool = call.name.as_str(),
                                output_len = output.len(),
                                "[agent_loop] tool succeeded"
                            );
                            scrub_credentials(&output)
                        } else {
                            tracing::warn!(
                                iteration,
                                tool = call.name.as_str(),
                                "[agent_loop] tool returned error: {output}"
                            );
                            format!("Error: {output}")
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::error!(
                            iteration,
                            tool = call.name.as_str(),
                            "[agent_loop] tool execution failed: {e}"
                        );
                        format!("Error executing {}: {e}", call.name)
                    }
                    Err(_) => {
                        tracing::error!(
                            iteration,
                            tool = call.name.as_str(),
                            secs = timeout_secs,
                            "[agent_loop] tool execution timed out"
                        );
                        format!(
                            "Error: tool '{}' timed out after {} seconds",
                            call.name, timeout_secs
                        )
                    }
                }
            } else {
                tracing::warn!(
                    iteration,
                    tool = call.name.as_str(),
                    "[agent_loop] unknown tool requested"
                );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::approval::ApprovalManager;
    use crate::openhuman::config::AutonomyConfig;
    use crate::openhuman::providers::traits::ProviderCapabilities;
    use crate::openhuman::providers::ChatResponse;
    use crate::openhuman::security::AutonomyLevel;
    use crate::openhuman::tools::{ToolResult, ToolScope};
    use async_trait::async_trait;
    use parking_lot::Mutex;

    struct ScriptedProvider {
        responses: Mutex<Vec<anyhow::Result<ChatResponse>>>,
        native_tools: bool,
        vision: bool,
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> Result<String> {
            Ok("fallback".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> Result<ChatResponse> {
            let mut guard = self.responses.lock();
            guard.remove(0)
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                native_tool_calling: self.native_tools,
                vision: self.vision,
                ..ProviderCapabilities::default()
            }
        }
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "echo"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::success("echo-out"))
        }
    }

    struct CliOnlyTool;

    #[async_trait]
    impl Tool for CliOnlyTool {
        fn name(&self) -> &str {
            "cli_only"
        }

        fn description(&self) -> &str {
            "cli only"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::success("should-not-run"))
        }

        fn scope(&self) -> ToolScope {
            ToolScope::CliRpcOnly
        }
    }

    struct ErrorResultTool;

    #[async_trait]
    impl Tool for ErrorResultTool {
        fn name(&self) -> &str {
            "error_result"
        }

        fn description(&self) -> &str {
            "error result"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::error("explicit failure"))
        }
    }

    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn name(&self) -> &str {
            "failing"
        }

        fn description(&self) -> &str {
            "failing"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            anyhow::bail!("boom")
        }
    }

    #[tokio::test]
    async fn run_tool_call_loop_rejects_vision_markers_for_non_vision_provider() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![]),
            native_tools: false,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("look [IMAGE:/tmp/x.png]")];

        let err = run_tool_call_loop(
            &provider,
            &mut history,
            &[],
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            1,
            None,
            None,
            &[],
        )
        .await
        .expect_err("vision markers should be rejected");

        assert!(err.to_string().contains("does not support vision input"));
    }

    #[tokio::test]
    async fn run_tool_call_loop_streams_final_text_chunks() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![Ok(ChatResponse {
                text: Some("word ".repeat(30)),
                tool_calls: vec![],
                usage: None,
            })]),
            native_tools: false,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("hello")];
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let result = run_tool_call_loop(
            &provider,
            &mut history,
            &[],
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            1,
            Some(tx),
            None,
            &[],
        )
        .await
        .expect("final text should succeed");

        let mut streamed = String::new();
        while let Some(chunk) = rx.recv().await {
            streamed.push_str(&chunk);
        }

        assert_eq!(result, streamed);
        assert!(history.iter().any(|msg| msg.role == "assistant"));
    }

    #[tokio::test]
    async fn run_tool_call_loop_blocks_cli_rpc_only_tools_in_prompt_mode() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                Ok(ChatResponse {
                    text: Some(
                        "<tool_call>{\"name\":\"cli_only\",\"arguments\":{}}</tool_call>".into(),
                    ),
                    tool_calls: vec![],
                    usage: None,
                }),
                Ok(ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                }),
            ]),
            native_tools: false,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("hello")];
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(CliOnlyTool)];

        let result = run_tool_call_loop(
            &provider,
            &mut history,
            &tools,
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            2,
            None,
            None,
            &[],
        )
        .await
        .expect("loop should recover after denial");

        assert_eq!(result, "done");
        let tool_results = history
            .iter()
            .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
            .expect("tool results should be appended");
        assert!(tool_results
            .content
            .contains("only available via explicit CLI/RPC invocation"));
    }

    #[tokio::test]
    async fn run_tool_call_loop_persists_native_tool_results_as_tool_messages() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                Ok(ChatResponse {
                    text: Some(String::new()),
                    tool_calls: vec![crate::openhuman::providers::ToolCall {
                        id: "call-1".into(),
                        name: "echo".into(),
                        arguments: "{}".into(),
                    }],
                    usage: None,
                }),
                Ok(ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                }),
            ]),
            native_tools: true,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("hello")];
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];

        let result = run_tool_call_loop(
            &provider,
            &mut history,
            &tools,
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            2,
            None,
            None,
            &[],
        )
        .await
        .expect("native tool flow should succeed");

        assert_eq!(result, "done");
        let tool_msg = history
            .iter()
            .find(|msg| msg.role == "tool")
            .expect("native tool result should be persisted");
        assert!(tool_msg.content.contains("\"tool_call_id\":\"call-1\""));
        assert!(tool_msg.content.contains("echo-out"));
    }

    #[tokio::test]
    async fn run_tool_call_loop_auto_approves_supervised_tools_on_non_cli_channels() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                Ok(ChatResponse {
                    text: Some(
                        "<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into(),
                    ),
                    tool_calls: vec![],
                    usage: None,
                }),
                Ok(ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                }),
            ]),
            native_tools: false,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("hello")];
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
        let approval = ApprovalManager::from_config(&AutonomyConfig {
            level: AutonomyLevel::Supervised,
            auto_approve: vec![],
            always_ask: vec!["echo".into()],
            ..AutonomyConfig::default()
        });

        let result = run_tool_call_loop(
            &provider,
            &mut history,
            &tools,
            "test-provider",
            "model",
            0.0,
            true,
            Some(&approval),
            "telegram",
            &crate::openhuman::config::MultimodalConfig::default(),
            2,
            None,
            None,
            &[],
        )
        .await
        .expect("non-cli channels should auto-approve supervised tools");

        assert_eq!(result, "done");
        let tool_results = history
            .iter()
            .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
            .expect("tool results should be appended");
        assert!(tool_results.content.contains("echo-out"));
        assert_eq!(approval.audit_log().len(), 1);
    }

    #[tokio::test]
    async fn run_tool_call_loop_reports_unknown_tool_and_uses_default_max_iterations() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                Ok(ChatResponse {
                    text: Some(
                        "<tool_call>{\"name\":\"missing\",\"arguments\":{}}</tool_call>".into(),
                    ),
                    tool_calls: vec![],
                    usage: None,
                }),
                Ok(ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                }),
            ]),
            native_tools: false,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("hello")];

        let result = run_tool_call_loop(
            &provider,
            &mut history,
            &[],
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            0,
            None,
            None,
            &[],
        )
        .await
        .expect("default iteration fallback should still succeed");

        assert_eq!(result, "done");
        let tool_results = history
            .iter()
            .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
            .expect("tool results should be appended");
        assert!(tool_results.content.contains("Unknown tool: missing"));
    }

    #[tokio::test]
    async fn run_tool_call_loop_formats_tool_error_paths() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                Ok(ChatResponse {
                    text: Some(
                        concat!(
                            "<tool_call>{\"name\":\"error_result\",\"arguments\":{}}</tool_call>",
                            "<tool_call>{\"name\":\"failing\",\"arguments\":{}}</tool_call>"
                        )
                        .into(),
                    ),
                    tool_calls: vec![],
                    usage: None,
                }),
                Ok(ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                }),
            ]),
            native_tools: false,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("hello")];
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(ErrorResultTool), Box::new(FailingTool)];

        let result = run_tool_call_loop(
            &provider,
            &mut history,
            &tools,
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            2,
            None,
            None,
            &[],
        )
        .await
        .expect("loop should recover after tool errors");

        assert_eq!(result, "done");
        let tool_results = history
            .iter()
            .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
            .expect("tool results should be appended");
        assert!(tool_results.content.contains("Error: explicit failure"));
        assert!(tool_results
            .content
            .contains("Error executing failing: boom"));
    }

    #[tokio::test]
    async fn run_tool_call_loop_propagates_provider_errors_and_max_iteration_failures() {
        let failing_provider = ScriptedProvider {
            responses: Mutex::new(vec![Err(anyhow::anyhow!("provider failed"))]),
            native_tools: false,
            vision: false,
        };
        let mut history = vec![ChatMessage::user("hello")];
        let err = run_tool_call_loop(
            &failing_provider,
            &mut history,
            &[],
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            1,
            None,
            None,
            &[],
        )
        .await
        .expect_err("provider error path should fail");
        assert!(err.to_string().contains("provider failed"));

        let looping_provider = ScriptedProvider {
            responses: Mutex::new(vec![Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
            })]),
            native_tools: false,
            vision: false,
        };
        let mut looping_history = vec![ChatMessage::user("hello")];
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
        let err = run_tool_call_loop(
            &looping_provider,
            &mut looping_history,
            &tools,
            "test-provider",
            "model",
            0.0,
            true,
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            1,
            None,
            None,
            &[],
        )
        .await
        .expect_err("loop should stop after configured iterations");
        assert!(err
            .to_string()
            .contains("Agent exceeded maximum tool iterations (1)"));
    }
}
