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
    crate::openhuman::security::live_policy::install(Arc::new(policy), std::env::temp_dir());

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
