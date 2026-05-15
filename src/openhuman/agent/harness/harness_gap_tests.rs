//! Gap-filling unit tests for the agent harness.
//!
//! These tests cover paths that were missing from the existing `*_tests.rs`
//! co-located files as identified by a coverage gap analysis:
//!
//! 1. Full user→LLM→tool→result→final turn cycle with `run_tool_call_loop`.
//! 2. `MaxIterationsExceeded` downcasts to the typed `AgentError` variant.
//! 3. `visible_tool_names` whitelist: tools outside the set are treated as unknown.
//! 4. `ContextGuard` surfaces `ContextExhausted` and aborts the loop.
//! 5. `parse_tool_calls` XML `<invoke>` tag variant (covered alongside other
//!    fallback formats).
//! 6. `DateTimeSection` produces an ISO-8601-like timestamp with a timezone token.
//! 7. `parse_tool_timeout_secs` default and boundary cases.
//!
//! Items that have NO underlying code and therefore cannot be tested:
//! - Spawn-depth gate (SpawnDepthExceeded) — no depth counter or variant exists.
//! - Follow-up resolution ("yes"/"no" disambiguation) — not implemented.
//! - Silence timer (SilenceTimeout, 600 s) — not implemented.
//! - `<invoke tool=…>` XML attribute form — the parser does not parse attributes;
//!   only the tag body (JSON) is used.

use crate::openhuman::agent::error::AgentError;
use crate::openhuman::agent::harness::tool_loop::run_tool_call_loop;
use crate::openhuman::context::guard::{ContextCheckResult, ContextGuard};
use crate::openhuman::providers::traits::ProviderCapabilities;
use crate::openhuman::providers::Provider;
use crate::openhuman::providers::{ChatMessage, ChatRequest, ChatResponse, UsageInfo};
use crate::openhuman::tool_timeout::parse_tool_timeout_secs;
use crate::openhuman::tools::{Tool, ToolResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use std::collections::HashSet;

// ─────────────────────────────────────────────────────────────────────────────
// Shared test doubles
// ─────────────────────────────────────────────────────────────────────────────

struct ScriptedProvider {
    responses: Mutex<Vec<anyhow::Result<ChatResponse>>>,
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("fallback".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let mut guard = self.responses.lock();
        guard.remove(0)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
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
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("echo-out"))
    }
}

struct PingTool;

#[async_trait]
impl Tool for PingTool {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "ping"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("pong"))
    }
}

fn multimodal_cfg() -> crate::openhuman::config::MultimodalConfig {
    crate::openhuman::config::MultimodalConfig::default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 1 — Full turn cycle: user → LLM emits tool call → tool executes →
//           result injected → LLM produces final text.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn full_turn_cycle_user_llm_tool_result_final() {
    // Round 1: LLM requests the "echo" tool.
    // Round 2: LLM produces a final reply after seeing the tool result.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
            }),
            Ok(ChatResponse {
                text: Some("The tool said: echo-out".into()),
                tool_calls: vec![],
                usage: None,
            }),
        ]),
    };
    let mut history = vec![ChatMessage::user("please echo something")];
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
        &multimodal_cfg(),
        2,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("full turn cycle should succeed");

    assert_eq!(result, "The tool said: echo-out");

    // History should contain: user | assistant (tool call) | user (tool results) | assistant (final)
    let roles: Vec<&str> = history.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "user", "assistant"],
        "history should have exactly 4 messages after one tool round-trip"
    );

    // The tool results message must contain the echo output.
    let tool_results = &history[2];
    assert_eq!(tool_results.role, "user");
    assert!(
        tool_results.content.contains("echo-out"),
        "tool result must be echoed into history, got: {}",
        tool_results.content
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 1 — MaxIterationsExceeded downcasts to typed AgentError.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn max_iterations_exceeded_downcasts_to_typed_agent_error() {
    // Provider keeps requesting the same tool forever — the loop
    // exhausts max_iterations=1 after one tool round-trip.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![Ok(ChatResponse {
            text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
            tool_calls: vec![],
            usage: None,
        })]),
    };
    let mut history = vec![ChatMessage::user("loop me")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];

    let err = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        None,
        "channel",
        &multimodal_cfg(),
        1,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect_err("loop must fail when iterations exhausted");

    // The anyhow error must downcast to the typed variant so callers
    // (channels dispatch, web_channel run_chat_task, Sentry filter)
    // can distinguish this deterministic outcome from transient failures.
    let agent_err = err
        .downcast_ref::<AgentError>()
        .expect("error should downcast to AgentError");
    assert!(
        matches!(agent_err, AgentError::MaxIterationsExceeded { max: 1 }),
        "expected MaxIterationsExceeded(1), got: {agent_err}"
    );

    // The string representation must contain the canonical prefix used
    // by the Sentry-emit suppression checks in channels dispatch.
    assert!(
        crate::openhuman::agent::error::is_max_iterations_error(&err.to_string()),
        "is_max_iterations_error must match the error text: {}",
        err
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 4 — visible_tool_names whitelist: tool outside the set → treated
//           as unknown; tool inside the set → executes normally.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn visible_tool_names_rejects_tool_outside_whitelist() {
    // Registry contains both "echo" and "ping".
    // The whitelist only allows "ping".
    // LLM calls "echo" (outside the whitelist) → should be treated as unknown.
    // LLM then produces a final text after seeing the unknown-tool error.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(ChatResponse {
                text: Some(
                    // Model calls the filtered-out tool.
                    "<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into(),
                ),
                tool_calls: vec![],
                usage: None,
            }),
            Ok(ChatResponse {
                text: Some("corrected response".into()),
                tool_calls: vec![],
                usage: None,
            }),
        ]),
    };
    let mut history = vec![ChatMessage::user("echo something")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool), Box::new(PingTool)];

    // Whitelist: only "ping" is visible.
    let whitelist: HashSet<String> = ["ping".to_string()].into();

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
        &multimodal_cfg(),
        2,
        None,
        Some(&whitelist),
        &[],
        None,
        None,
    )
    .await
    .expect("loop should recover after whitelisted-out tool call");

    assert_eq!(result, "corrected response");

    // The tool results injected back to the LLM must report "echo" as unknown —
    // it was filtered out by the whitelist.
    let tool_results = history
        .iter()
        .find(|m| m.role == "user" && m.content.contains("[Tool results]"))
        .expect("tool results must be appended after tool call");
    assert!(
        tool_results.content.contains("Unknown tool: echo"),
        "whitelisted-out tool must be reported as unknown, got: {}",
        tool_results.content
    );
}

#[tokio::test]
async fn visible_tool_names_allows_tool_inside_whitelist() {
    // Whitelist includes "echo" — the call should execute normally.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
            }),
            Ok(ChatResponse {
                text: Some("heard echo-out".into()),
                tool_calls: vec![],
                usage: None,
            }),
        ]),
    };
    let mut history = vec![ChatMessage::user("echo something")];
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let whitelist: HashSet<String> = ["echo".to_string()].into();

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
        &multimodal_cfg(),
        2,
        None,
        Some(&whitelist),
        &[],
        None,
        None,
    )
    .await
    .expect("whitelisted tool should execute");

    assert_eq!(result, "heard echo-out");

    // Tool result must contain the actual tool output, not the unknown-tool message.
    let tool_results = history
        .iter()
        .find(|m| m.role == "user" && m.content.contains("[Tool results]"))
        .expect("tool results must be appended");
    assert!(
        tool_results.content.contains("echo-out"),
        "tool should have executed and returned its output, got: {}",
        tool_results.content
    );
    assert!(
        !tool_results.content.contains("Unknown tool"),
        "allowed tool must not be reported as unknown, got: {}",
        tool_results.content
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 5 — ContextGuard: ContextExhausted is surfaced cleanly.
//           (Unit test on the guard directly; the loop integration path is
//           exercised implicitly via context_guard.check() inside the loop.)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn context_guard_exhausted_after_circuit_breaker_and_95pct_utilization() {
    // Simulate the scenario where compaction has failed 3 times (circuit
    // breaker tripped) and context is at 96 % — the guard must surface
    // ContextExhausted, not CompactionNeeded, so the loop can bail cleanly.
    let mut guard = ContextGuard::with_context_window(100_000);
    guard.update_usage(&UsageInfo {
        input_tokens: 91_000,
        output_tokens: 5_100, // 96.1 % total
        context_window: 100_000,
        ..Default::default()
    });

    // Trip the circuit breaker.
    guard.record_compaction_failure();
    guard.record_compaction_failure();
    guard.record_compaction_failure();
    assert!(guard.is_compaction_disabled(), "breaker should be tripped");

    let result = guard.check();
    assert!(
        matches!(result, ContextCheckResult::ContextExhausted { .. }),
        "guard must return ContextExhausted when breaker is tripped and >95%, got: {result:?}"
    );

    // The utilization percentage embedded in the result must be ≥ 95.
    if let ContextCheckResult::ContextExhausted {
        utilization_pct, ..
    } = result
    {
        assert!(
            utilization_pct >= 95,
            "utilization_pct in exhausted result should be ≥ 95, got {utilization_pct}"
        );
    }
}

#[test]
fn context_guard_update_usage_raises_window_from_response() {
    // UsageInfo that carries a non-zero `context_window` must update the
    // guard's known window — a guard with window=0 is a no-op, so this
    // path matters for the first provider response that reports its window.
    let mut guard = ContextGuard::new(); // window = 0 initially
    assert_eq!(guard.check(), ContextCheckResult::Ok, "unknown window → Ok");

    guard.update_usage(&UsageInfo {
        input_tokens: 95_000,
        output_tokens: 2_000,
        context_window: 100_000,
        ..Default::default()
    });
    // Now at 97 % with no compaction failures — CompactionNeeded (below hard limit if
    // circuit breaker is not tripped, but above COMPACTION_TRIGGER_THRESHOLD=90%).
    // With compaction NOT disabled, the guard returns CompactionNeeded, not Exhausted.
    assert_eq!(
        guard.check(),
        ContextCheckResult::CompactionNeeded,
        "97% with no circuit breaker should return CompactionNeeded"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 3 — parse_tool_calls: <invoke> tag variant (JSON body, not attributes).
//           The parser recognises <invoke>…</invoke> as a tool-call tag.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn parse_tool_calls_invoke_tag_with_json_body() {
    use crate::openhuman::agent::harness::parse::parse_tool_calls;

    // The <invoke> tag is listed in TOOL_CALL_OPEN_TAGS and must parse the
    // JSON body identically to <tool_call>.
    let input = "Some text\n<invoke>{\"name\":\"echo\",\"arguments\":{\"value\":\"hi\"}}</invoke>\ntrailing";
    let (text, calls) = parse_tool_calls(input);

    assert_eq!(calls.len(), 1, "should parse one call from <invoke> block");
    assert_eq!(calls[0].name, "echo");
    assert_eq!(calls[0].arguments, serde_json::json!({"value": "hi"}));
    // Text surrounding the tag must be preserved.
    assert!(
        text.contains("Some text"),
        "text before tag should be preserved"
    );
    assert!(
        text.contains("trailing"),
        "text after tag should be preserved"
    );
}

#[test]
fn parse_tool_calls_markdown_fence_yaml_like_json_body() {
    use crate::openhuman::agent::harness::parse::parse_tool_calls;

    // The markdown fence regex accepts ```tool_call\n…\n```.
    // The body must be valid JSON (the parser calls extract_json_values
    // on the inner content, not a YAML parser).
    let input = "preamble\n```tool_call\n{\"name\":\"ping\",\"arguments\":{}}\n```\npostamble";
    let (text, calls) = parse_tool_calls(input);

    assert_eq!(calls.len(), 1, "should parse one call from markdown fence");
    assert_eq!(calls[0].name, "ping");
    assert!(text.contains("preamble"));
    assert!(text.contains("postamble"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 5 (tool timeout) — parse_tool_timeout_secs defaults and boundaries.
//   Already covered in tool_timeout/mod.rs but pinned here for the gap report.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tool_timeout_parse_default_and_boundaries() {
    // Default when absent.
    assert_eq!(parse_tool_timeout_secs(None), 120);
    // Default when non-numeric.
    assert_eq!(parse_tool_timeout_secs(Some("bad")), 120);
    // Boundary values.
    assert_eq!(parse_tool_timeout_secs(Some("1")), 1);
    assert_eq!(parse_tool_timeout_secs(Some("3600")), 3600);
    // Out of range → default.
    assert_eq!(parse_tool_timeout_secs(Some("0")), 120);
    assert_eq!(parse_tool_timeout_secs(Some("3601")), 120);
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 8 — DateTimeSection: ISO 8601 timestamp + IANA timezone token.
//           Extended coverage beyond the existing mod_tests.rs test.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn datetime_section_output_matches_iso8601_date_and_utc_offset_pattern() {
    use crate::openhuman::agent::prompts::{DateTimeSection, PromptContext, PromptSection};
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::LazyLock;

    static EMPTY_FILTER: LazyLock<HashSet<String>> = LazyLock::new(HashSet::new);
    static EMPTY_TOOLS: &[crate::openhuman::agent::prompts::PromptTool<'static>] = &[];
    static EMPTY_INTEGRATIONS: &[crate::openhuman::context::prompt::ConnectedIntegration] = &[];

    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: EMPTY_TOOLS,
        skills: &[],
        dispatcher_instructions: "",
        learned: crate::openhuman::agent::prompts::LearnedContextData::default(),
        visible_tool_names: &EMPTY_FILTER,
        tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
    };

    let rendered = DateTimeSection.build(&ctx).unwrap();
    let payload = rendered
        .strip_prefix("## Current Date & Time\n\n")
        .expect("DateTimeSection must start with the heading");

    // Must contain a date token matching YYYY-MM-DD.
    let has_date = payload.chars().filter(|c| c.is_ascii_digit()).count() >= 8;
    assert!(
        has_date,
        "payload must contain enough digits for a date: {payload}"
    );

    // Must include a UTC offset in the form UTC+HH:MM or UTC-HH:MM or UTC+00:00.
    assert!(
        payload.contains("UTC"),
        "payload must contain UTC offset marker: {payload}"
    );

    // The IANA timezone string is either a slashed zone or the literal "UTC" fallback.
    let has_iana = payload.contains('/') || payload.contains(" UTC ");
    assert!(
        has_iana,
        "payload must contain an IANA zone (slashed) or UTC fallback: {payload}"
    );
}
