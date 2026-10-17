//! Native turn-model source coverage.

use std::sync::Arc;

use super::*;

#[test]
fn crate_native_turn_source_retains_only_role_and_config() {
    let source = TurnModelSource::new_crate_native(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        Arc::new(crate::config::Config::default()),
    );

    assert!(source.direct_model.is_none());
    assert!(source.crate_native.is_some());
}

#[test]
fn direct_model_turn_source_builds_without_provider_adapter() {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "done",
        ]));
    let source = TurnModelSource::from_model(model);

    assert!(source.crate_native.is_none());
    assert!(source.direct_model.is_some());

    let models = source
        .build("mock-model", 0.0, Some(32_000), None)
        .expect("direct model source builds");
    assert_eq!(models.provider_id(), "injected");
    assert_eq!(models.context_window(), Some(32_000));
    assert!(!models.native_tools());
}

#[test]
fn run_policy_for_makes_invalid_tool_arguments_recoverable() {
    let policy = run_policy_for(10, false);
    assert_eq!(
        policy.invalid_args,
        InvalidArgsPolicy::NormalizeThenReturnToolError,
        "schema-invalid calls must be normalized, then return a corrective tool result instead of aborting the turn"
    );
}

/// Stand-in with `mcp_registry_tool_call`'s exact parameter schema; records
/// the arguments it actually executes with.
struct NestedArgsTool(std::sync::Mutex<Vec<serde_json::Value>>);

#[async_trait::async_trait]
impl tinytools::Tool for NestedArgsTool {
    fn name(&self) -> &str {
        "mcp_registry_tool_call"
    }

    fn description(&self) -> &str {
        "call an MCP server tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "server_id": { "type": "string" },
                "tool_name": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["server_id", "tool_name"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        self.0.lock().unwrap().push(arguments);
        Ok(tinytools::ToolResult::success("ok"))
    }
}

fn scripted_response(
    tool_calls: Vec<tinyinference_llm::tool::ToolCall>,
    text: &str,
) -> tinyinference_llm::model::ModelResponse {
    use tinyinference_llm::message::{AssistantMessage, ContentBlock};
    let content = if text.is_empty() {
        Vec::new()
    } else {
        vec![ContentBlock::Text(text.to_string())]
    };
    let finish = if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    tinyinference_llm::model::ModelResponse {
        message: AssistantMessage {
            id: None,
            content,
            tool_calls,
            usage: None,
            origin: None,
        },
        usage: None,
        finish_reason: Some(finish.to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
        correlation: None,
        resolved_route: None,
    }
}

/// Regression: DeepSeek sent `mcp_registry_tool_call` with a JSON-encoded
/// nested `"arguments": "{}"`. Under the turn policy that string must be
/// decoded to an object and the tool must run, instead of every call in the
/// batch failing validation with "arguments.arguments must be object, got string".
#[tokio::test]
async fn run_policy_decodes_stringified_nested_object_arguments() {
    use tinyinference_llm::message::Message;
    use tinyinference_llm::tool::ToolCall;

    let call = |id: &str, tool: &str| {
        ToolCall::new(
            id,
            "mcp_registry_tool_call",
            serde_json::json!({
                "arguments": "{}",
                "server_id": "21385eb2",
                "tool_name": tool,
            }),
        )
    };
    let model = Arc::new(tinyagents_harness::testkit::ScriptedModel::new(vec![
        scripted_response(
            vec![call("call-1", "list_projects"), call("call-2", "list_tags")],
            "",
        ),
        scripted_response(Vec::new(), "done"),
    ]));
    let tool = Arc::new(NestedArgsTool(std::sync::Mutex::new(Vec::new())));

    let mut harness: tinyagents_harness::runtime::AgentHarness<()> =
        tinyagents_harness::runtime::AgentHarness::new();
    harness.register_model("mock", model);
    harness.register_tool(tool.clone());
    harness.with_policy(run_policy_for(10, false));

    let run = harness
        .invoke_default(&(), vec![Message::user("organize my ticktick")])
        .await
        .expect("turn completes");

    assert_eq!(run.final_response.unwrap().text(), "done");
    let executed = tool.0.lock().unwrap().clone();
    assert_eq!(executed.len(), 2, "both batched calls must execute");
    for args in &executed {
        assert_eq!(args["arguments"], serde_json::json!({}));
    }
    assert!(
        !run.messages
            .iter()
            .any(|m| format!("{m:?}").contains("must be object, got string")),
        "no validation error may reach the transcript"
    );
}

#[test]
fn run_policy_retries_one_nontruncated_empty_completion() {
    let policy = run_policy_for(10, false);
    assert_eq!(policy.empty_response_retries, 1);
}

#[test]
fn run_policy_wires_both_wall_clock_ceilings_from_their_resolvers() {
    // Compare against the harness resolvers fed the same environment, so a
    // dev/CI environment exporting either override cannot fail the test while
    // the wiring is correct.
    let policy = run_policy_for(10, false);
    assert_eq!(
        policy.limits.max_model_call_ms,
        tinyagents_harness::turn_policy::model_call_wall_clock_ms(
            std::env::var("OPENHUMAN_MODEL_CALL_TIMEOUT_SECS")
                .ok()
                .as_deref(),
            false
        )
    );
    assert_eq!(
        policy.limits.max_wall_clock_ms,
        crate::agent::tinyagents::turn_policy::agent_turn_wall_clock_ms_for(false)
    );
}

/// #6413: the retry schedule must outlast a throttled upstream.
///
/// These pin the two numbers that were wrong — the attempt count and the total
/// time spent waiting — rather than "it retried", which passed on the old
/// 2-retry/1.5 s schedule and would pass again if someone shortened it.
#[test]
fn run_policy_retry_schedule_outlasts_a_throttled_upstream() {
    use tinyagents_harness::retry::JITTER_FRACTION;

    let policy = run_policy_for(10, false);
    let retry = &policy.retry;

    // 4 retries (5 attempts). The issue asks for "at least 3".
    assert_eq!(retry.max_attempts, 5, "attempts (first try + retries)");
    assert!(
        retry.backoff_sleep,
        "a retry that does not wait is not a backoff"
    );

    // The harness applies `max_attempts_capped_at(limits.max_retries_per_call)`
    // at the model call, and `RunLimits` defaults that to 3 — so without this the
    // policy above is silently clamped to 4 attempts and the change is a no-op.
    // This is the assertion that catches a "fix" that raised only `max_attempts`.
    assert!(
        policy.limits.max_retries_per_call + 1 >= retry.max_attempts,
        "max_attempts {} is clamped by max_retries_per_call {} — the schedule would \
         silently lose an attempt",
        retry.max_attempts,
        policy.limits.max_retries_per_call,
    );

    // Total time actually spent waiting, on the WORST jitter draw. `rand01 = 0.0`
    // is the bottom of the band (`base * (1 - JITTER_FRACTION)`), which is the
    // figure that has to clear the issue's floor — the nominal schedule clearing
    // it while the floor does not would be an intermittent failure to retry long
    // enough, and those are the ones nobody reproduces.
    let retries = retry.max_attempts - 1;
    let worst: std::time::Duration = (0..retries)
        .map(|attempt| retry.backoff_for_attempt_with(attempt, 0.0))
        .sum();
    assert!(
        worst >= std::time::Duration::from_secs(30),
        "worst-case backoff across {retries} retries is {:?}; #6413 requires >= 30s",
        worst,
    );

    // And the nominal (mid-band) schedule, so a future edit that satisfies the
    // floor by widening jitter rather than lengthening the curve still fails.
    let nominal: std::time::Duration = (0..retries)
        .map(|attempt| retry.backoff_for_attempt_with(attempt, 0.5))
        .sum();
    assert!(
        nominal >= std::time::Duration::from_secs(40),
        "nominal backoff is {nominal:?}; expected >= 40s so the jittered floor has margin",
    );

    // Jitter on: a throttled fleet must not retry in lockstep on one curve.
    assert!(
        retry.jitter,
        "identical curves keep a saturated upstream saturated"
    );
    const {
        assert!(JITTER_FRACTION > 0.0, "jitter must actually widen the band");
    }

    // Both ceilings stay bounded — this must not become an unbounded retry
    // (cf. #6412, the opposite failure in another crate).
    assert!(retry.max_attempts < 10, "attempts must stay bounded");
    assert!(
        retry.max_retry_after_ms > 0 && retry.max_retry_after_ms <= 300_000,
        "a server Retry-After must be honoured but capped, not obeyed without limit"
    );
}
