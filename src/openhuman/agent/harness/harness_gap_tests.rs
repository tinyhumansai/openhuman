//! Gap-filling unit tests for the agent harness.
//!
//! These tests cover paths that were missing from the existing `*_tests.rs`
//! co-located files as identified by a coverage gap analysis:
//!
//! 1. Full user→LLM→tool→result→final turn cycle — now covered by the
//!    tinyagents route's tests (`src/openhuman/tinyagents/tests.rs`), which
//!    exercise `run_turn_via_tinyagents_shared` end to end.
//! 2. `MaxIterationsExceeded` downcasts to the typed `AgentError` variant.
//! 3. `visible_tool_names` whitelist: tools outside the set are treated as unknown.
//! 4. `ContextGuard` surfaces `ContextExhausted` and aborts the loop.
//! 5. `parse_tool_calls` XML `<invoke>` tag variant (covered alongside other
//!    fallback formats).
//! 6. `DateTimeSection` produces an ISO-8601-like timestamp with a timezone token.
//! 7. `parse_tool_timeout_secs` default and boundary cases.
//! 8. Spawn-depth gate (`SpawnDepthExceeded`) is covered in
//!    `subagent_runner/ops_tests.rs` because it lives at the `run_subagent`
//!    boundary.
//!
//! Items that have NO underlying code and therefore cannot be tested:
//! - Follow-up resolution ("yes"/"no" disambiguation) — not implemented.
//! - Silence timer (SilenceTimeout, 600 s) — not implemented.
//! - `<invoke tool=…>` XML attribute form — the parser does not parse attributes;
//!   only the tag body (JSON) is used.

use crate::openhuman::inference::provider::traits::ProviderCapabilities;
use crate::openhuman::inference::provider::Provider;
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse};
use crate::openhuman::tool_timeout::parse_tool_timeout_secs;
use crate::openhuman::tools::{Tool, ToolResult};
use async_trait::async_trait;
use parking_lot::Mutex;

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

fn multimodal_file_cfg() -> crate::openhuman::config::MultimodalFileConfig {
    crate::openhuman::config::MultimodalFileConfig::default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 1 — Full turn cycle: user → LLM emits tool call → tool executes →
//           result injected → LLM produces final text.
// ─────────────────────────────────────────────────────────────────────────────

// NOTE: The `ContextGuard`/`ContextCheckResult` tests that used to live here
// (context_guard_exhausted_after_circuit_breaker_and_95pct_utilization,
// context_guard_update_usage_raises_window_from_response) were removed during the
// tinyagents migration: the context reducer shell (`context/guard.rs`) was
// deleted (commit d55ea9a5d) and the tested API no longer exists.

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

// ─────────────────────────────────────────────────────────────────────────────
// Item 3b — parse_tool_calls: Claude-native <invoke name="…"> attribute form
//           with nested <parameter name="…"> children (issue #3493).
//           Claude-family models ignore the injected <tool_call>{json} template
//           and emit their trained syntax; the parser must recover it instead of
//           leaking the raw markup as assistant text.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn parse_tool_calls_invoke_attribute_form_single_param() {
    use crate::openhuman::agent::harness::parse::parse_tool_calls;

    let input =
        "Sure.\n<invoke name=\"echo\">\n<parameter name=\"value\">hi</parameter>\n</invoke>\ndone";
    let (text, calls) = parse_tool_calls(input);

    assert_eq!(
        calls.len(),
        1,
        "attribute-form <invoke> should parse one call"
    );
    assert_eq!(calls[0].name, "echo");
    assert_eq!(calls[0].arguments, serde_json::json!({"value": "hi"}));
    // Surrounding prose preserved; raw <invoke> markup must not leak.
    assert!(text.contains("Sure."), "text before tag preserved");
    assert!(text.contains("done"), "text after tag preserved");
    assert!(
        !text.contains("<invoke"),
        "raw <invoke> markup must not surface in assistant text"
    );
    assert!(
        !text.contains("<parameter"),
        "raw <parameter> markup must not surface in assistant text"
    );
}

#[test]
fn parse_tool_calls_invoke_attribute_form_multiple_params_scalar_policy() {
    use crate::openhuman::agent::harness::parse::parse_tool_calls;

    // Multiple <parameter> children. Scalar policy: a value that parses as JSON
    // (number, bool) becomes that JSON type; anything else stays a string. A
    // parameter with an empty name is skipped (it cannot key an argument).
    let input = concat!(
        "<invoke name=\"search\">\n",
        "<parameter name=\"query\">rust parsers</parameter>\n",
        "<parameter name=\"limit\">5</parameter>\n",
        "<parameter name=\"fuzzy\">true</parameter>\n",
        "<parameter name=\"\">ignored</parameter>\n",
        "</invoke>"
    );
    let (_text, calls) = parse_tool_calls(input);

    assert_eq!(calls.len(), 1, "should parse one call");
    assert_eq!(calls[0].name, "search");
    assert_eq!(
        calls[0].arguments,
        serde_json::json!({"query": "rust parsers", "limit": 5, "fuzzy": true})
    );
}

#[test]
fn parse_tool_calls_invoke_attribute_form_missing_close_tag_is_text() {
    use crate::openhuman::agent::harness::parse::parse_tool_calls;

    // No closing </invoke>: nothing to dispatch. The block is left as text
    // rather than silently dropped.
    let input = "before\n<invoke name=\"echo\">\n<parameter name=\"v\">hi</parameter>";
    let (text, calls) = parse_tool_calls(input);

    assert_eq!(calls.len(), 0, "unterminated <invoke> yields no calls");
    assert!(text.contains("before"), "preceding text preserved");
    assert!(
        text.contains("<invoke"),
        "unterminated block left as text, not dropped"
    );
}

#[test]
fn parse_tool_calls_invoke_attribute_form_missing_name_is_text() {
    use crate::openhuman::agent::harness::parse::parse_tool_calls;

    // Attribute form without a `name` attribute cannot name a tool → no call.
    let input = "<invoke foo=\"bar\">\n<parameter name=\"v\">hi</parameter>\n</invoke>";
    let (_text, calls) = parse_tool_calls(input);

    assert_eq!(calls.len(), 0, "missing name attribute yields no calls");
}

#[test]
fn parse_tool_calls_mixed_tool_call_json_and_invoke_attribute() {
    use crate::openhuman::agent::harness::parse::parse_tool_calls;

    // A canonical <tool_call>{json} block and a Claude-native attribute-form
    // <invoke> block in the same response are both recovered, earliest first.
    let input = concat!(
        "<tool_call>{\"name\":\"first\",\"arguments\":{\"a\":1}}</tool_call>\n",
        "<invoke name=\"second\">\n<parameter name=\"b\">two</parameter>\n</invoke>"
    );
    let (_text, calls) = parse_tool_calls(input);

    assert_eq!(calls.len(), 2, "both tag forms parsed");
    assert_eq!(calls[0].name, "first");
    assert_eq!(calls[0].arguments, serde_json::json!({"a": 1}));
    assert_eq!(calls[1].name, "second");
    assert_eq!(calls[1].arguments, serde_json::json!({"b": "two"}));
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
// Item 8 — Current-time grounding (#3602). The volatile timestamp now rides the
//           per-turn user message via `current_datetime_line` (so a long-lived
//           session's frozen prompt prefix can't go stale); `DateTimeSection`
//           carries only the static grounding *rule*. Pin both halves.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn current_datetime_line_matches_iso8601_date_and_utc_offset_pattern() {
    // The per-turn stamp is the one carrying the concrete clock — assert its
    // ISO-8601 date, UTC offset, and IANA zone (or `UTC` fallback).
    let payload = crate::openhuman::agent::prompts::current_datetime_line();

    // Parse the concrete `YYYY-MM-DD HH:MM:SS` prefix rather than counting
    // loose digits, so a malformed layout can't slip through.
    let rest = payload
        .strip_prefix("Current Date & Time: ")
        .expect("stamp must start with the canonical prefix");
    let dt = rest
        .get(0..19)
        .expect("stamp must include YYYY-MM-DD HH:MM:SS");
    chrono::NaiveDateTime::parse_from_str(dt, "%Y-%m-%d %H:%M:%S")
        .expect("timestamp must match YYYY-MM-DD HH:MM:SS");
    assert!(
        payload.contains("UTC"),
        "stamp must contain UTC offset marker: {payload}"
    );
    let has_iana = payload.contains('/') || payload.contains(" UTC ");
    assert!(
        has_iana,
        "stamp must contain an IANA zone (slashed) or UTC fallback: {payload}"
    );
}
