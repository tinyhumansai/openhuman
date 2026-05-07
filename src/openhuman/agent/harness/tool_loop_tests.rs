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
        None,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        Some(&summarizer),
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
        None,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        1,
        None,
        None,
        &[],
        None,
        None,
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
        None,
        None,
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
        None,
        None,
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
        None,
        None,
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
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
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
        None,
        None,
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
                text: Some("<tool_call>{\"name\":\"missing\",\"arguments\":{}}</tool_call>".into()),
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
        None,
        None,
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
        None,
        None,
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
        None,
        None,
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
        None,
        None,
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
            }),
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
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
            None,
            "channel",
            &crate::openhuman::config::MultimodalConfig::default(),
            10,
            None,
            None,
            &[],
            None,
            None,
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
        None,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        1,
        None,
        None,
        &[],
        None,
        None,
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
            }),
            // Round 2: stop.
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
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
        None,
        "channel",
        &crate::openhuman::config::MultimodalConfig::default(),
        2,
        None,
        None,
        &[],
        None,
        None,
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
        &tool_results.content[..tool_results.content.len().min(200)]
    );
    assert!(
        tool_results.content.len() < 1_000,
        "raw 200k payload should not appear in history (got {} bytes)",
        tool_results.content.len()
    );
}
