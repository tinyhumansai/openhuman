use super::*;
use crate::openhuman::inference::provider::traits::ProviderCapabilities;
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse};
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

/// Simulates a delegated sub-agent (`run_code` / `tools_agent`) whose provider
/// call hit a permanent, non-retryable wall: `dispatch_subagent` converts that
/// sub-agent `Err` into a `ToolResult::error` carrying the budget-exhaustion
/// body. Used to prove the #3104 cascade halts on the FIRST such failure.
struct BudgetExhaustedDelegationTool;

#[async_trait]
impl Tool for BudgetExhaustedDelegationTool {
    fn name(&self) -> &str {
        "run_code"
    }

    fn description(&self) -> &str {
        "delegate to the code executor (always 400s on budget here)"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        // Mirrors dispatch.rs::format_subagent_failure wrapping the upstream body.
        Ok(ToolResult::error(
            "run_code failed and did not complete — no work was performed. \
             Error: OpenHuman API error (400): {\"error\":\"Insufficient budget\"}",
        ))
    }
}

/// Records whether it ever executed. Used to prove that a tool placed AFTER a
/// terminal-inference failure in the SAME assistant-message batch never runs
/// (#3104 — Codex review #3779, batched tool calls).
struct RanTrackerTool {
    ran: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl Tool for RanTrackerTool {
    fn name(&self) -> &str {
        "ran_tracker"
    }

    fn description(&self) -> &str {
        "records that it executed"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(ToolResult::success("ran-tracker-out"))
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

/// Tool that emits a large payload (~150 KB), used to exercise the
/// payload-summarizer interception path in the integration test
/// below.
struct BigPayloadTool;

#[async_trait]
impl Tool for BigPayloadTool {
    fn name(&self) -> &str {
        "big_payload"
    }

    fn description(&self) -> &str {
        "emits a 150 KB payload to trigger summarization"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        // 150 KB of payload — well above the 100 KB default threshold.
        Ok(ToolResult::success("X".repeat(150_000)))
    }
}

/// Mock summarizer that always returns a fixed compressed string,
/// used to verify that [`run_tool_call_loop`] swaps the raw tool
/// output for the summary before pushing it into history.
struct MockSummarizer {
    summary: String,
}

#[async_trait]
impl super::super::payload_summarizer::PayloadSummarizer for MockSummarizer {
    async fn maybe_summarize(
        &self,
        _tool_name: &str,
        _parent_task_hint: Option<&str>,
        raw: &str,
    ) -> Result<Option<super::super::payload_summarizer::SummarizedPayload>> {
        Ok(Some(super::super::payload_summarizer::SummarizedPayload {
            summary: self.summary.clone(),
            original_bytes: raw.len(),
            summary_bytes: self.summary.len(),
        }))
    }
}

#[tokio::test]
async fn run_tool_call_loop_intercepts_oversized_tool_results_via_summarizer() {
    // Provider scripts a single tool call to `big_payload`, then a
    // final "done" message after the tool result lands in history.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(ChatResponse {
                text: Some(
                    "<tool_call>{\"name\":\"big_payload\",\"arguments\":{}}</tool_call>".into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("dump the data")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(BigPayloadTool)];
    let summarizer = MockSummarizer {
        summary: "compressed-summary-marker".to_string(),
    };

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        Some(&summarizer),
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("loop with summarizer should succeed");

    assert_eq!(result, "done");

    // The summarized marker should be present in the appended
    // tool-results message; the raw 150 KB blob of 'X' should NOT.
    let tool_results = history
        .iter()
        .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
        .expect("tool results should be appended");
    assert!(
        tool_results.content.contains("compressed-summary-marker"),
        "summarizer output should replace the raw payload in history"
    );
    // 150 KB of "X" is much larger than the summary; if it slipped
    // through, the message body would be enormous.
    assert!(
        tool_results.content.len() < 10_000,
        "raw 150 KB payload must not appear in history (got {} bytes)",
        tool_results.content.len()
    );
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        1,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
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
            reasoning_content: None,
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        1,
        Some(tx),
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
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
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
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
                tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                    id: "call-1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                    extra_content: None,
                }],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
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

/// Behavioral end-to-end test of the `resolve_time` fix through the *real*
/// agent tool loop. The model is scripted only to *decide* to call
/// `resolve_time` (the part a live LLM does); the loop then dispatches to the
/// real registered tool, executes it, and threads the result back into the
/// conversation — exactly the path that was broken when the integrations
/// agent hand-computed "24h ago" as a ~10-month-wrong epoch. We assert the
/// resolved value the next turn would consume is the *correct* timestamp.
#[tokio::test]
async fn run_tool_call_loop_executes_real_resolve_time_and_threads_back_correct_epoch() {
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            // Turn 1: the model asks to resolve the caller's window. This is
            // the only thing we script — the value comes from the real tool.
            Ok(ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                    id: "call-rt".into(),
                    name: "resolve_time".into(),
                    arguments: "{\"expr\":\"2026-06-09T19:12:00Z\"}".into(),
                    extra_content: None,
                }],
                usage: None,
                reasoning_content: None,
            }),
            // Turn 2: model wraps up once it has the resolved value.
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        native_tools: true,
        vision: false,
    };
    let mut history = vec![ChatMessage::user(
        "messages since 2026-06-09T19:12:00Z please",
    )];
    // The REAL tool, resolved + executed by the loop (not a stub).
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(crate::openhuman::tools::ResolveTimeTool::new())];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("resolve_time tool flow should succeed");

    assert_eq!(result, "done");
    let tool_msg = history
        .iter()
        .find(|msg| msg.role == "tool")
        .expect("resolve_time result should be persisted as a tool message");
    // The correct epoch for 2026-06-09T19:12:00Z. The real incident's agent
    // hand-computed this as 1752189120 (2025-07-10) — ~10 months wrong — and
    // fetched the wrong Slack window. The tool returns the right value, ready
    // for the next turn to pass as `oldest`.
    assert!(
        tool_msg.content.contains("1781032320"),
        "expected the correctly resolved unix_s in the tool result; got: {}",
        tool_msg.content
    );
    assert!(
        tool_msg.content.contains("1781032320.000000"),
        "expected the slack_ts representation in the tool result; got: {}",
        tool_msg.content
    );
    // The ~10-month-wrong value the unfixed path produced must never appear.
    assert!(
        !tool_msg.content.contains("1752189120"),
        "tool result must not contain the miscomputed epoch"
    );
}

#[tokio::test]
async fn run_tool_call_loop_reports_unknown_tool_and_uses_default_max_iterations() {
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"missing\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        0,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
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
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        1,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect_err("provider error path should fail");
    assert!(err.to_string().contains("provider failed"));

    let looping_provider = ScriptedProvider {
        responses: Mutex::new(vec![Ok(ChatResponse {
            text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
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
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        1,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect_err("loop should stop after configured iterations");
    assert!(err
        .to_string()
        .contains("Agent exceeded maximum tool iterations (1)"));
}

#[tokio::test]
async fn run_tool_call_loop_aborts_when_stop_hook_returns_stop() {
    use crate::openhuman::agent::stop_hooks::{with_stop_hooks, StopDecision, StopHook, TurnState};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Stops the loop on the second iteration (1-based).
    struct StopOnIteration(Arc<AtomicU32>);

    #[async_trait]
    impl StopHook for StopOnIteration {
        fn name(&self) -> &str {
            "test-iter-cap"
        }
        async fn check(&self, ctx: &TurnState<'_>) -> StopDecision {
            self.0.store(ctx.iteration, Ordering::Relaxed);
            if ctx.iteration >= 2 {
                StopDecision::Stop {
                    reason: "tripped on iter 2".into(),
                }
            } else {
                StopDecision::Continue
            }
        }
    }

    // Provider would happily loop forever — first response asks for a
    // tool, second response would too (we never reach it because the
    // stop hook fires at the top of iteration 2).
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("loop me")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let last_seen = Arc::new(AtomicU32::new(0));
    let hook: Arc<dyn StopHook> = Arc::new(StopOnIteration(last_seen.clone()));

    let err = with_stop_hooks(vec![hook], async {
        run_tool_call_loop(
            &provider,
            &mut history,
            &tools,
            "test-provider",
            "model",
            0.0,
            true,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            &crate::openhuman::config::MultimodalFileConfig::default(),
            10,
            None,
            None,
            &[],
            None,
            None,
            &crate::openhuman::tools::policy::DefaultToolPolicy,
        )
        .await
    })
    .await
    .expect_err("stop hook should abort the loop");

    assert!(
        err.to_string().contains("stopped by hook 'test-iter-cap'"),
        "got: {err}"
    );
    assert!(
        err.to_string().contains("tripped on iter 2"),
        "stop reason should be propagated, got: {err}"
    );
    assert_eq!(
        last_seen.load(Ordering::Relaxed),
        2,
        "hook should have observed iteration 2"
    );
}

#[tokio::test]
async fn run_tool_call_loop_runs_unchanged_when_no_stop_hooks_installed() {
    // Sanity: with no `with_stop_hooks` scope, the loop behaves
    // identically to before this feature landed.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![Ok(ChatResponse {
            text: Some("done".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("hi")];
    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &[],
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        1,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("loop should succeed without stop hooks");
    assert_eq!(result, "done");
}

#[tokio::test]
async fn run_tool_call_loop_applies_per_tool_max_result_size_cap() {
    /// Tool that emits a 200k-char body and declares a 100-char cap
    /// via `max_result_size_chars`. The loop should truncate before
    /// threading the body into history.
    struct CappedHugeTool;

    #[async_trait]
    impl Tool for CappedHugeTool {
        fn name(&self) -> &str {
            "capped_huge"
        }
        fn description(&self) -> &str {
            "emits a giant body but caps itself"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::success("Z".repeat(200_000)))
        }
        fn permission_level(&self) -> crate::openhuman::tools::PermissionLevel {
            crate::openhuman::tools::PermissionLevel::ReadOnly
        }
        fn max_result_size_chars(&self) -> Option<usize> {
            Some(100)
        }
    }

    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            // Round 1: ask for the tool.
            Ok(ChatResponse {
                text: Some(
                    "<tool_call>{\"name\":\"capped_huge\",\"arguments\":{}}</tool_call>".into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Round 2: stop.
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("call the tool")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(CappedHugeTool)];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("loop with capped tool should succeed");
    assert_eq!(result, "done");

    // Tool-results message should contain the truncation marker and
    // be far smaller than the 200k raw body (the 100-char cap plus a
    // small marker, well under 1k bytes total for this one call).
    let tool_results = history
        .iter()
        .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
        .expect("tool results should be appended to history");
    assert!(
        tool_results.content.contains("[truncated by tool cap:"),
        "expected truncation marker, got body: {}",
        crate::openhuman::util::utf8_safe_prefix_at_byte_boundary(&tool_results.content, 200)
    );
    assert!(
        tool_results.content.len() < 1_000,
        "raw 200k payload should not appear in history (got {} bytes)",
        tool_results.content.len()
    );
}

/// Repeated-failure circuit breaker: when the model re-issues the IDENTICAL
/// failing call, the loop must halt early with a root-cause summary instead of
/// grinding to `max_iterations` and returning `MaxIterationsExceeded`.
#[tokio::test]
async fn run_tool_call_loop_halts_on_repeated_identical_failure() {
    // Script the same `error_result` call (identical args) far more times than
    // the REPEAT_FAILURE_THRESHOLD (3); the loop should stop after the 3rd.
    let mut responses: Vec<anyhow::Result<ChatResponse>> = Vec::new();
    for _ in 0..10 {
        responses.push(Ok(ChatResponse {
            text: Some(
                "<tool_call>{\"name\":\"error_result\",\"arguments\":{}}</tool_call>".into(),
            ),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        }));
    }
    let provider = ScriptedProvider {
        responses: Mutex::new(responses),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("install the thing")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(ErrorResultTool)];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        10, // max_iterations — must NOT be reached; breaker fires at 3
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("repeated-failure halt returns Ok with a root-cause summary, not an error");

    assert!(
        result.contains("Stopping") && result.contains("retried 3 times"),
        "expected an early repeated-failure halt summary, got: {result}"
    );
    assert!(
        result.contains("explicit failure"),
        "halt summary should embed the underlying error, got: {result}"
    );
    // Breaker fired at the 3rd identical failure → only 3 of the 10 scripted
    // responses consumed (7 remain). Proves it did NOT grind to max_iterations.
    assert_eq!(
        provider.responses.lock().len(),
        7,
        "loop should consume exactly 3 LLM turns before halting"
    );
}

/// No-progress circuit breaker: even with VARIED arguments (so no single
/// signature repeats), a run of back-to-back failures with zero success halts
/// once it hits NO_PROGRESS_FAILURE_THRESHOLD (6).
#[tokio::test]
async fn run_tool_call_loop_halts_when_no_progress() {
    let mut responses = Vec::new();
    for i in 0..10 {
        // Distinct args each turn → per-signature count stays at 1, so only the
        // consecutive-failure guard can trip.
        responses.push(Ok(ChatResponse {
            text: Some(format!(
                "<tool_call>{{\"name\":\"error_result\",\"arguments\":{{\"i\":{i}}}}}</tool_call>"
            )),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        }));
    }
    let provider = ScriptedProvider {
        responses: Mutex::new(responses),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("keep trying")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(ErrorResultTool)];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        20,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("no-progress halt returns Ok with a summary");

    assert!(
        result.contains("Stopping") && result.contains("in a row failed"),
        "expected a no-progress halt summary, got: {result}"
    );
    // Fires at the 6th consecutive failure → 6 of 10 responses consumed.
    assert_eq!(
        provider.responses.lock().len(),
        4,
        "loop should consume exactly 6 LLM turns before halting on no-progress"
    );
}

/// #3104 end-to-end repro through the real engine: a delegation that fails with
/// a PERMANENT inference condition (out of budget) must halt the orchestrator on
/// the FIRST failure with an actionable root cause — NOT grind to the
/// 6-consecutive no-progress backstop (the Plan → Run Code ×6 → Tools Agent ×2
/// cascade the user saw). Before the fix this loop consumed 6 LLM turns and
/// ended in a generic message; after it, exactly 1.
#[tokio::test]
async fn run_tool_call_loop_halts_on_first_budget_exhausted_delegation() {
    let mut responses = Vec::new();
    for i in 0..10 {
        // Varied args so the per-signature repeat guard can NEVER trip — only the
        // terminal-inference check (or, pre-fix, the 6-consecutive backstop) can.
        responses.push(Ok(ChatResponse {
            text: Some(format!(
                "<tool_call>{{\"name\":\"run_code\",\"arguments\":{{\"step\":{i}}}}}</tool_call>"
            )),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        }));
    }
    let provider = ScriptedProvider {
        responses: Mutex::new(responses),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("build me a dashboard")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(BudgetExhaustedDelegationTool)];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        10, // max_iterations — must NOT be reached; terminal halt fires at 1
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("budget-exhausted halt returns Ok with an actionable summary");

    assert!(
        result.contains("Stopping") && result.contains("out of inference budget"),
        "expected an actionable budget halt, got: {result}"
    );
    // The decisive assertion: only the FIRST scripted turn was consumed (9 of 10
    // remain). Proves the cascade is killed on the first permanent failure
    // instead of burning 6 doomed, paid delegations.
    assert_eq!(
        provider.responses.lock().len(),
        9,
        "loop must halt after exactly 1 turn on a permanent inference failure"
    );
}

/// #3104 batched-tool-call guarantee (Codex review #3779): when a single
/// assistant message emits MULTIPLE tool calls and the FIRST records a terminal
/// inference failure (out of budget / provider-config), the loop must STOP
/// executing the rest of the batch — so a second delegated call in the same
/// message can never launch a paid sub-agent after the first proved the wall is
/// unrecoverable. Pre-fix the loop set `halt_reason` but drained the batch first.
#[tokio::test]
async fn run_tool_call_loop_stops_batch_after_first_terminal_failure() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // One assistant message with TWO tool calls: the budget-exhausted delegation
    // first, a tracker second. Native mode emits both as `tool_calls` in a single
    // ChatResponse so they share one batch iteration.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![Ok(ChatResponse {
            text: Some(String::new()),
            tool_calls: vec![
                crate::openhuman::inference::provider::ToolCall {
                    id: "call-budget".into(),
                    name: "run_code".into(),
                    arguments: "{}".into(),
                    extra_content: None,
                },
                crate::openhuman::inference::provider::ToolCall {
                    id: "call-tracker".into(),
                    name: "ran_tracker".into(),
                    arguments: "{}".into(),
                    extra_content: None,
                },
            ],
            usage: None,
            reasoning_content: None,
        })]),
        native_tools: true,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("build me a dashboard")];
    let ran = Arc::new(AtomicBool::new(false));
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(BudgetExhaustedDelegationTool),
        Box::new(RanTrackerTool { ran: ran.clone() }),
    ];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        5,
        None,
        None,
        &[],
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("budget-exhausted halt returns Ok with an actionable summary");

    assert!(
        result.contains("Stopping") && result.contains("out of inference budget"),
        "expected an actionable budget halt, got: {result}"
    );
    // The decisive assertion: the SECOND call in the batch must NOT have run.
    assert!(
        !ran.load(Ordering::SeqCst),
        "a tool placed after a terminal inference failure in the same batch must NOT execute"
    );
}

// -- RepeatFailureGuard (shared by run_tool_call_loop + run_inner_loop) --------

#[test]
fn repeat_failure_guard_halts_on_3_identical() {
    let mut g = RepeatFailureGuard::new();
    assert!(g
        .record("shell", "pip install yfinance", false, "err")
        .is_none());
    assert!(g
        .record("shell", "pip install yfinance", false, "err")
        .is_none());
    let halt = g.record(
        "shell",
        "pip install yfinance",
        false,
        "externally-managed-environment",
    );
    assert!(halt.is_some(), "same call failing 3x must trip the breaker");
    assert!(halt.unwrap().contains("externally-managed-environment"));
}

#[test]
fn repeat_failure_guard_halts_on_6_consecutive_varied() {
    let mut g = RepeatFailureGuard::new();
    // Distinct signatures → repeat guard never trips; only the consecutive run does.
    for i in 0..5 {
        assert!(g.record("shell", &format!("cmd{i}"), false, "e").is_none());
    }
    assert!(
        g.record("shell", "cmd5", false, "e").is_some(),
        "6 consecutive failures must trip the no-progress guard"
    );
}

#[test]
fn repeat_failure_guard_success_resets_consecutive() {
    let mut g = RepeatFailureGuard::new();
    for i in 0..5 {
        g.record("shell", &format!("cmd{i}"), false, "e");
    }
    assert!(
        g.record("shell", "ok", true, "fine").is_none(),
        "success returns None"
    );
    // After a success the consecutive counter is back to 0, so one more failure
    // is nowhere near the 6-in-a-row threshold.
    assert!(g.record("shell", "cmd6", false, "e").is_none());
}

// -- Hard policy rejects (marker-driven, halt on first verbatim repeat) ---------

#[test]
fn hard_reject_kind_detects_markers() {
    use crate::openhuman::security::{POLICY_BLOCKED_MARKER, POLICY_DENIED_MARKER};
    // Marker survives the `Error: …` wrapping the tool/subagent layers add.
    assert_eq!(
        hard_reject_kind(&format!("Error: {POLICY_BLOCKED_MARKER} Path not allowed")),
        Some(HardReject::Blocked)
    );
    assert_eq!(
        hard_reject_kind(&format!("{POLICY_DENIED_MARKER} User denied 'shell'.")),
        Some(HardReject::Denied)
    );
    assert_eq!(hard_reject_kind("Error: connection reset by peer"), None);
}

#[test]
fn hard_reject_blocked_halts_on_first_repeat_not_third() {
    use crate::openhuman::security::POLICY_BLOCKED_MARKER;
    let mut g = RepeatFailureGuard::new();
    let blocked =
        format!("Error: {POLICY_BLOCKED_MARKER} Path not allowed by security policy: /etc");
    // First occurrence is allowed through so the model can read the reason and pivot.
    assert!(
        g.record("file_read", "/etc/passwd", false, &blocked)
            .is_none(),
        "first hard reject should not halt — let the model change approach"
    );
    // Second identical attempt = first verbatim repeat → halt (vs the generic 3).
    let halt = g.record("file_read", "/etc/passwd", false, &blocked);
    assert!(
        halt.is_some(),
        "an identical blocked call must halt on the 2nd attempt"
    );
    let msg = halt.unwrap();
    assert!(msg.contains("blocked by the security policy"), "got: {msg}");
}

#[test]
fn hard_reject_denied_halts_on_first_repeat() {
    use crate::openhuman::security::POLICY_DENIED_MARKER;
    let mut g = RepeatFailureGuard::new();
    let denied = format!("Error: {POLICY_DENIED_MARKER} User denied 'shell' execution.");
    assert!(g.record("shell", "rm -rf build", false, &denied).is_none());
    let halt = g.record("shell", "rm -rf build", false, &denied);
    assert!(
        halt.is_some(),
        "re-issued denied call must halt on the 2nd attempt"
    );
    assert!(halt.unwrap().contains("denied and re-issued"));
}

#[test]
fn hard_reject_distinct_args_do_not_trip_repeat() {
    use crate::openhuman::security::POLICY_BLOCKED_MARKER;
    let mut g = RepeatFailureGuard::new();
    let mk = POLICY_BLOCKED_MARKER;
    // Different forbidden paths each time: the per-signature repeat guard never
    // trips (every signature is seen once); only the no-progress backstop can.
    for i in 0..5 {
        assert!(g
            .record(
                "file_read",
                &format!("/etc/x{i}"),
                false,
                &format!("{mk} blocked")
            )
            .is_none());
    }
    assert!(
        g.record("file_read", "/etc/x5", false, &format!("{mk} blocked"))
            .is_some(),
        "6 distinct hard rejects in a row should still trip the no-progress guard"
    );
}

// -- Terminal inference failures (#3104: budget / provider-config cascade) -------

#[test]
fn terminal_inference_failure_kind_classifies_budget_and_config() {
    // Budget wins precedence; provider-config covers the user-state model/key
    // rejections; transient / 5xx / generic-4xx match NEITHER (so retryable
    // failures keep their normal consecutive-failure grace).
    assert_eq!(
        terminal_inference_failure_kind(
            "run_code failed and did not complete. Error: OpenHuman API error (400): \
             {\"error\":\"insufficient budget — add credits\"}"
        ),
        Some(TerminalInferenceFailure::BudgetExhausted)
    );
    assert_eq!(
        terminal_inference_failure_kind(
            "tools_agent failed and did not complete. Error: ollama API error (400 Bad Request): \
             {\"error\":{\"message\":\"\\\"bge-m3:latest\\\" does not support chat\"}}"
        ),
        Some(TerminalInferenceFailure::ProviderConfig)
    );
    for transient in [
        "Error: connection reset by peer",
        "Error: 503 Service Unavailable",
        "Error: rate limit exceeded, retry after 1s",
        "run_code failed and did not complete. Error: timed out",
    ] {
        assert_eq!(
            terminal_inference_failure_kind(transient),
            None,
            "{transient:?} must NOT classify as a terminal inference failure"
        );
    }
}

#[test]
fn terminal_inference_failure_requires_delegated_inference_envelope() {
    // Codex review #3779: the message-only provider classifiers match short
    // substrings (`invalid temperature`, `model field is required`,
    // `insufficient balance`, …) that can legitimately appear in a RECOVERABLE
    // tool's stderr. Those must NOT trip the terminal halt — only a result that
    // carries a delegated-inference/provider envelope may.

    // 1) Recoverable `shell`/`run_code` SCRIPT stderr that merely *contains* a
    //    classifier substring — no provider envelope → must NOT classify, so the
    //    normal consecutive-failure grace still applies and the script can be
    //    retried/fixed.
    for recoverable in [
        // A Python test raising on a `temperature` arg — not an inference call.
        "ValueError: invalid temperature: only 1 is allowed for this model",
        // A user script validating config and printing the provider-ish phrase.
        "AssertionError: model field is required",
        // A finance script echoing an account-balance string.
        "RuntimeError: insufficient balance in wallet 0xabc",
        // A test asserting on the literal remediation copy.
        "FAILED test_models.py::test_unknown - assert 'model_not_found' in resp",
    ] {
        assert_eq!(
            terminal_inference_failure_kind(recoverable),
            None,
            "recoverable tool stderr without an inference envelope must NOT classify \
             as a terminal inference failure: {recoverable:?}"
        );
    }

    // 2) The SAME phrases, but now wrapped in a genuine delegated-inference
    //    envelope (provider HTTP error / reliable rollup / sub-agent dispatch
    //    wrapper) → MUST classify, because these only come from a delegated
    //    provider round-trip.
    assert_eq!(
        terminal_inference_failure_kind(
            "run_code failed and did not complete. Error: custom_openai API error \
             (400 Bad Request): {\"error\":{\"message\":\"invalid temperature: only 1 is \
             allowed for this model\"}}"
        ),
        Some(TerminalInferenceFailure::ProviderConfig),
        "a delegated provider config-rejection (with envelope) must classify"
    );
    assert_eq!(
        terminal_inference_failure_kind(
            "tools_agent failed and did not complete. Error: OpenHuman API error (402): \
             {\"error\":\"Insufficient balance\"}"
        ),
        Some(TerminalInferenceFailure::BudgetExhausted),
        "a delegated budget-exhaustion (with envelope) must classify"
    );
    // The reliable-chain rollup envelope (no `API error` token) also qualifies.
    assert_eq!(
        terminal_inference_failure_kind(
            "All providers/models failed. Attempts:\n provider=custom_openai model=gpt-5.5 \
             attempt 1/3: non_retryable; error=insufficient balance"
        ),
        Some(TerminalInferenceFailure::BudgetExhausted),
        "a reliable-chain exhaustion rollup carrying a budget body must classify"
    );
}

#[test]
fn bare_provider_api_error_in_tool_stderr_does_not_classify() {
    // Codex review #3779 (follow-up): a recoverable `shell`/`run_code` task that
    // is debugging its OWN API client can print the verbatim provider-HTTP
    // envelope shape (`<provider> API error (status): …`). Without a
    // harness-generated wrapper (`failed and did not complete` /
    // `all providers/models failed` / `may not be available on your provider`)
    // that output must NOT be treated as a delegated inference failure — else the
    // whole turn halts after a single failed command with a misleading "fix your
    // model in Settings → AI" message.
    for tool_stderr in [
        // The exact shape called out in the review: a config-rejection body that
        // a debugging script could print to stdout/stderr.
        "OpenAI API error (400): invalid temperature",
        "OpenAI API error (400): model field is required",
        // Budget-flavoured body, same forged-envelope risk.
        "custom_openai API error (402 Payment Required): {\"error\":\"Insufficient balance\"}",
        // Responses/streaming envelope variants, still bare (no harness wrapper).
        "custom_openai Responses API error: {\"error\":{\"message\":\"model 'x' not found\"}}",
        "custom_openai streaming API error (404 Not Found): model 'llama3.3' not found",
    ] {
        assert_eq!(
            terminal_inference_failure_kind(tool_stderr),
            None,
            "a bare provider-HTTP envelope without a harness wrapper must NOT \
             classify as a terminal inference failure: {tool_stderr:?}"
        );
    }
}

#[test]
fn bare_provider_api_error_keeps_consecutive_failure_grace() {
    // The behavioural consequence of the above at the guard level: a `run_code`
    // task printing a bare provider API-error body on each attempt must keep its
    // normal consecutive-failure grace (it is recoverable user code, not a
    // delegated wall), and only trip the 6-in-a-row no-progress backstop — never
    // halt on the FIRST failure.
    let mut g = RepeatFailureGuard::new();
    let bare = "OpenAI API error (400): invalid temperature";
    for i in 0..(NO_PROGRESS_FAILURE_THRESHOLD - 1) {
        assert!(
            g.record("run_code", &format!("debug-attempt{i}"), false, bare)
                .is_none(),
            "a bare provider API-error in tool stderr must NOT halt on failure #{}",
            i + 1
        );
    }
    // The Nth consecutive failure trips the generic no-progress backstop, as for
    // any other recoverable failure — proving the grace was preserved, not that
    // the terminal classifier fired.
    let halt = g.record(
        "run_code",
        &format!("debug-attempt{}", NO_PROGRESS_FAILURE_THRESHOLD - 1),
        false,
        bare,
    );
    let msg = halt.expect("the consecutive no-progress backstop still trips");
    assert!(
        msg.contains("in a row failed with no progress"),
        "must halt via the generic no-progress path, not the terminal-inference \
         path: {msg}"
    );
    assert!(
        !msg.contains("Settings → AI") && !msg.contains("out of inference budget"),
        "must NOT use the terminal-inference remediation copy: {msg}"
    );
}

#[test]
fn delegated_provider_failure_still_halts_first_after_narrowing() {
    // Boundary guard for the #3779 narrowing: a GENUINE delegated provider
    // failure — the same config-rejection body, but wrapped by the sub-agent
    // dispatch (`failed and did not complete`) — MUST still halt on the first
    // occurrence. Narrowing the envelope must not regress the #3104 fix.
    let mut g = RepeatFailureGuard::new();
    let delegated = "run_code failed and did not complete — no work was performed. \
                     Error: OpenAI API error (400): invalid temperature";
    let halt = g.record("run_code", "{\"prompt\":\"x\"}", false, delegated);
    let msg = halt.expect("a wrapped delegated provider-config rejection must halt first");
    assert!(msg.contains("rejected the request"), "got: {msg}");
    assert!(
        msg.contains("Settings → AI"),
        "actionable remediation: {msg}"
    );
}

#[test]
fn terminal_budget_failure_halts_on_first_occurrence() {
    let mut g = RepeatFailureGuard::new();
    let budget = "run_code failed and did not complete — no work was performed. \
                  Error: OpenHuman API error (400): {\"error\":\"Insufficient budget\"}";
    let halt = g.record("run_code", "{\"prompt\":\"write a file\"}", false, budget);
    assert!(
        halt.is_some(),
        "a budget-exhausted delegation must halt on the FIRST failure, not grind to 6"
    );
    let msg = halt.unwrap();
    assert!(msg.contains("out of inference budget"), "got: {msg}");
    assert!(msg.contains("`run_code`"), "names the failing step: {msg}");
}

#[test]
fn terminal_config_rejection_halts_on_first_occurrence() {
    let mut g = RepeatFailureGuard::new();
    let cfg = "tools_agent failed and did not complete. Error: OpenHuman API error \
               (400 Bad Request): Model 'gpt-5.5' is not available. Use GET \
               /openai/v1/models to list available models.";
    let halt = g.record("tools_agent", "{\"prompt\":\"do the thing\"}", false, cfg);
    assert!(
        halt.is_some(),
        "a provider-config rejection must halt on the FIRST failure"
    );
    let msg = halt.unwrap();
    assert!(msg.contains("rejected the request"), "got: {msg}");
    assert!(
        msg.contains("Settings → AI"),
        "actionable remediation: {msg}"
    );
}

#[test]
fn terminal_failure_halts_first_even_across_varied_delegation_tools() {
    // The #3104 cascade reproduction at the guard level: the orchestrator
    // re-emits a doomed step under VARIED tool names (plan → run_code →
    // tools_agent), so neither the identical-(tool,args) repeat guard nor the
    // 6-consecutive no-progress guard would catch it in time. The terminal
    // classifier must halt on the very first permanent failure regardless of
    // which delegation tool surfaced it.
    let mut g = RepeatFailureGuard::new();
    // Carries the sub-agent dispatch envelope (`failed and did not complete`)
    // so it is recognised as a *delegated* inference failure, not arbitrary
    // tool stderr — see `has_inference_failure_envelope`.
    let budget = "plan failed and did not complete. Error: {\"error\":\"insufficient balance\"}";
    let halt = g.record("plan", "{\"goal\":\"x\"}", false, budget);
    assert!(
        halt.is_some(),
        "first permanent failure under ANY delegation tool must halt immediately"
    );
}

#[test]
fn terminal_classifier_does_not_short_circuit_transient_grace() {
    // A genuinely transient (retryable) failure must still flow through the
    // existing no-progress backstop — proving the terminal halt is additive,
    // not a blanket "halt on first failure" regression.
    let mut g = RepeatFailureGuard::new();
    for i in 0..5 {
        assert!(
            g.record(
                "run_code",
                &format!("attempt{i}"),
                false,
                "Error: timed out"
            )
            .is_none(),
            "transient failures must NOT halt before the 6-consecutive threshold"
        );
    }
    assert!(
        g.record("run_code", "attempt5", false, "Error: timed out")
            .is_some(),
        "6 consecutive transient failures still trip the no-progress guard"
    );
}

/// Provider that records the tool-spec names of every `chat()` request
/// it sees, then returns the next scripted response.
struct CapturingProvider {
    /// One entry per `chat()` call — the tool-name list extracted from
    /// `ChatRequest.tools`. `None` if `tools` was `None`.
    captured: Mutex<Vec<Option<Vec<String>>>>,
    responses: Mutex<Vec<anyhow::Result<ChatResponse>>>,
    native_tools: bool,
}

#[async_trait]
impl Provider for CapturingProvider {
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
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        let names = request
            .tools
            .map(|specs| specs.iter().map(|s| s.name.clone()).collect::<Vec<_>>());
        self.captured.lock().push(names);
        let mut guard = self.responses.lock();
        guard.remove(0)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: self.native_tools,
            vision: false,
            ..ProviderCapabilities::default()
        }
    }
}

#[tokio::test]
async fn run_tool_call_loop_dedups_duplicate_tool_names_before_provider_call() {
    // Provider returns a single final text response — no tool calls —
    // so the loop terminates after exactly one `chat()` invocation,
    // and the captured tool list reflects what the fix is supposed to
    // guard against (no duplicate names reaching the wire).
    let provider = CapturingProvider {
        captured: Mutex::new(Vec::new()),
        responses: Mutex::new(vec![Ok(ChatResponse {
            text: Some("done".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        // Native tool-calling on: only when the provider supports native
        // tools does `run_tool_call_loop` populate `ChatRequest.tools`.
        native_tools: true,
    };

    // Registry has `EchoTool` (name = "echo"). `extra_tools` adds a
    // second tool also named "echo" — the exact collision pattern from
    // the bug report (a synthesised delegation tool whose
    // `delegate_name` shadows a same-named skill tool).
    let registry: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let extra: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];

    let mut history = vec![ChatMessage::user("hi")];
    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &registry,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        &crate::openhuman::config::MultimodalFileConfig::default(),
        2,
        None,
        None,
        &extra,
        None,
        None,
        &crate::openhuman::tools::policy::DefaultToolPolicy,
    )
    .await
    .expect("loop should succeed with deduplicated tool list");
    assert_eq!(result, "done");

    let captured = provider.captured.lock();
    assert_eq!(
        captured.len(),
        1,
        "exactly one chat() call expected for a final-only response"
    );
    let names = captured[0]
        .as_ref()
        .expect("native_tools=true should populate ChatRequest.tools");
    let echo_count = names.iter().filter(|n| n.as_str() == "echo").count();
    assert_eq!(
        echo_count, 1,
        "duplicate tool names must be dropped before the provider call \
         (TAURI-RUST-4) — got names={:?}",
        names
    );
}

// ── End-to-end: agent loop → ApprovalGate → auto_approve short-circuit ──
//
// Exercises the real seam: a scripted LLM emits a tool call for an
// external-effect tool, the loop routes it through the process-global
// `ApprovalGate` (`try_global`), and the tool's presence on the
// `auto_approve` "Always allow" list short-circuits the gate to `Allow`
// *before* parking — so the tool executes without a prompt, even though a
// chat context is present (which would otherwise park it).

/// A tool with an external side effect, so the loop gates it via the
/// `ApprovalGate`. Records whether `execute` actually ran.
struct ExternalEffectTool {
    ran: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl Tool for ExternalEffectTool {
    fn name(&self) -> &str {
        "ext_effect_e2e_tool"
    }
    fn description(&self) -> &str {
        "external effect (e2e gate test)"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        true
    }
    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(ToolResult::success("did-external-effect"))
    }
}

#[tokio::test]
async fn auto_approved_external_effect_tool_runs_through_loop_without_parking() {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    // Serialize live-policy / gate global access against the other tests that
    // install or reload them (gate auto_approve test, live_policy test, autonomy
    // ops tests) — all take this same lock.
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let tool_name = "ext_effect_e2e_tool";

    // Always-allow the tool via the live policy the gate reads.
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve: vec![tool_name.into()],
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    crate::openhuman::security::live_policy::install(
        Arc::new(policy),
        std::env::temp_dir(),
        std::env::temp_dir(),
    );

    // Install the process-global gate so the loop's external-effect branch has a
    // gate to route through (idempotent; the loop calls `ApprovalGate::try_global`).
    let cfg = crate::openhuman::config::Config {
        workspace_dir: std::env::temp_dir(),
        ..crate::openhuman::config::Config::default()
    };
    crate::openhuman::approval::ApprovalGate::init_global(cfg, "session-loop-gate-e2e");

    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(ChatResponse {
                text: Some(format!(
                    "<tool_call>{{\"name\":\"{tool_name}\",\"arguments\":{{}}}}</tool_call>"
                )),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        native_tools: false,
        vision: false,
    };
    let mut history = vec![ChatMessage::user("please act")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(ExternalEffectTool { ran: ran.clone() })];

    // Run *inside* a chat context: without the allowlist the gate would park
    // this external-effect call — so a clean completion proves the auto_approve
    // shortcut (checked before chat-context parking) is what let it through.
    let result = crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
        .scope(
            crate::openhuman::approval::ApprovalChatContext {
                thread_id: "t-e2e".into(),
                client_id: "c-e2e".into(),
            },
            run_tool_call_loop(
                &provider,
                &mut history,
                &tools,
                "test-provider",
                "model",
                0.0,
                true,
                "channel",
                &crate::openhuman::config::MultimodalConfig::default(),
                &crate::openhuman::config::MultimodalFileConfig::default(),
                2,
                None,
                None,
                &[],
                None,
                None,
                &crate::openhuman::tools::policy::DefaultToolPolicy,
            ),
        )
        .await
        .expect("loop should complete without parking on an auto-approved tool");

    assert_eq!(result, "done");
    assert!(
        ran.load(Ordering::SeqCst),
        "auto-approved external-effect tool must execute (gate must not park it)"
    );
}

#[test]
fn repeat_output_guard_trips_at_threshold() {
    let mut g = RepeatOutputGuard::new();
    // The first THRESHOLD-1 identical signatures must NOT trip.
    for _ in 1..REPEAT_OUTPUT_THRESHOLD {
        assert!(g.record("same-narration|run_code|{args}").is_none());
    }
    // The THRESHOLD-th identical signature trips with a no-progress summary.
    let halt = g
        .record("same-narration|run_code|{args}")
        .expect("identical streak at threshold must trip");
    assert!(
        halt.contains("IDENTICAL") || halt.contains("stuck") || halt.contains("progress"),
        "halt summary should explain the no-progress loop: {halt}"
    );
}

#[test]
fn repeat_output_guard_resets_on_changed_signature() {
    let mut g = RepeatOutputGuard::new();
    assert!(g.record("a").is_none());
    assert!(g.record("a").is_none());
    // A different signature = real progress; the streak resets.
    assert!(g.record("b").is_none());
    // It then takes a FULL fresh streak of the new signature to trip — so
    // interleaved/varied work never trips it.
    let mut last = None;
    for _ in 1..REPEAT_OUTPUT_THRESHOLD {
        last = g.record("b");
    }
    assert!(
        last.is_some(),
        "a fresh THRESHOLD-long identical streak should trip after a reset"
    );
}
