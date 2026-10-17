//! Tool-span attributes: the classified failure class and the model whose
//! call requested the tool.

use super::*;

// ── tool spans: failure class + model attribution ───────────────────────────

fn failed_tool_completed(call_id: &str, tool: &str, output: &str) -> AgentProgress {
    AgentProgress::ToolCallCompleted {
        call_id: call_id.to_string(),
        tool_name: tool.to_string(),
        success: false,
        output_chars: output.chars().count(),
        output: output.to_string(),
        arguments: None,
        elapsed_ms: 5,
        iteration: 1,
        failure: Some(tinytools::status::classify(output, false)),
        display_label: None,
        display_detail: None,
        structured: None,
    }
}

/// A plain non-zero command exit and a harness failure both close as an error
/// span, so the class rides the span (content-free, so not gated on capture):
/// it is what separates "the program exited 1" from "the harness failed" in
/// Langfuse, and a plain exit is exported at WARNING instead of ERROR.
#[test]
fn failed_tool_span_carries_its_failure_class() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 3,
            },
            1,
        ),
        (tool_started("c1", "shell", 1), 2),
        (
            failed_tool_completed(
                "c1",
                "shell",
                "Command failed (exit code 1)\n[stderr]\nollama: timed out",
            ),
            3,
        ),
        (tool_started("c2", "shell", 1), 4),
        (
            failed_tool_completed(
                "c2",
                "shell",
                "invalid arguments for tool `shell`: missing `command`; expected schema: {}",
            ),
            5,
        ),
        (tool_started("c3", "web_search", 1), 6),
        (tool_completed("c3", "web_search", true, 3, 1), 7),
    ]);
    c.finish(100);
    let class_of = |call: &str| {
        c.spans()
            .iter()
            .find(|s| s.attributes.get("tool.call_id") == Some(&serde_json::json!(call)))
            .and_then(|s| s.attributes.get("tool.failure_class").cloned())
    };
    assert_eq!(class_of("c1"), Some(serde_json::json!("CommandFailed")));
    assert_eq!(class_of("c2"), Some(serde_json::json!("InvalidArguments")));
    assert_eq!(class_of("c3"), None, "a successful call carries no class");
    let level_of = |call: &str| {
        c.spans()
            .iter()
            .find(|s| s.attributes.get("tool.call_id") == Some(&serde_json::json!(call)))
            .and_then(|s| s.attributes.get("observation.level").cloned())
    };
    // A program that exited non-zero exports at WARNING (Langfuse showed 27%
    // of `shell` calls as errors, ~70% of them ordinary non-zero exits); a
    // harness failure stays ERROR.
    assert_eq!(level_of("c1"), Some(serde_json::json!("WARNING")));
    assert_eq!(level_of("c2"), None);
}

/// Tool observations name the model whose call requested them, so tool errors
/// correlate to a model without joining against the generation span.
#[test]
fn tool_spans_carry_the_model_that_requested_them() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 3,
            },
            1,
        ),
        (tool_started("early", "shell", 1), 2),
        (tool_completed("early", "shell", true, 1, 1), 3),
        (model_call("chat-v1", 0, 0), 4),
        (tool_started("c1", "shell", 1), 5),
        (
            failed_tool_completed("c1", "shell", "Command failed (exit code 2)"),
            6,
        ),
        (spawn("task-1", "Researcher"), 7),
        (
            AgentProgress::SubagentIterationStarted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                iteration: 1,
                max_iterations: 8,
                extended_policy: false,
            },
            8,
        ),
        (
            AgentProgress::ModelCallCompleted {
                model: "scout-v2".to_string(),
                provider_id: "openai".to_string(),
                subagent_task_id: Some("task-1".to_string()),
                input: None,
                output: None,
                iteration: 1,
                input_tokens: 10,
                output_tokens: 1,
                cached_input_tokens: 0,
                cache_creation_tokens: 0,
                reasoning_tokens: 0,
                cost_usd: Some(0.0),
            },
            9,
        ),
        (
            AgentProgress::SubagentToolCallStarted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "file_read".to_string(),
                arguments: serde_json::Value::Null,
                iteration: 1,
                display_label: None,
                display_detail: None,
            },
            10,
        ),
        (
            AgentProgress::SubagentToolCallCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "file_read".to_string(),
                success: false,
                output_chars: 0,
                output: String::new(),
                arguments: None,
                elapsed_ms: 1,
                iteration: 1,
                failure: Some(tinytools::status::classify(
                    "Command failed (exit code 1)",
                    false,
                )),
                display_label: None,
                display_detail: None,
                structured: None,
            },
            11,
        ),
        (tool_started("c2", "shell", 2), 12),
    ]);
    c.finish(100);
    let attr = |call: &str, key: &str| {
        c.spans()
            .iter()
            .find(|s| s.attributes.get("tool.call_id") == Some(&serde_json::json!(call)))
            .and_then(|s| s.attributes.get(key).cloned())
    };
    assert_eq!(attr("early", "tool.model"), None, "no model call seen yet");
    assert_eq!(
        attr("c1", "tool.model"),
        Some(serde_json::json!("managed.chat-v1"))
    );
    assert_eq!(
        attr("sc-1", "tool.model"),
        Some(serde_json::json!("openai.scout-v2")),
        "a child tool names the child's model, not the parent's"
    );
    assert_eq!(
        attr("sc-1", "tool.failure_class"),
        Some(serde_json::json!("CommandFailed")),
        "a child tool span carries its failure class too"
    );
    assert_eq!(
        attr("c2", "tool.model"),
        Some(serde_json::json!("managed.chat-v1")),
        "the parent keeps its own model after a child ran"
    );
}
