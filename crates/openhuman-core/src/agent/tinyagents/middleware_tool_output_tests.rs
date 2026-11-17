use crate::agent::tinyagents::middleware::tool_output::ToolOutputMiddleware;
// Imported here rather than in the middleware module since #6014: the
// production install site moved to `install_turn_middleware` (which names it
// fully-qualified), so the module itself no longer references the type.
use super::*;
use crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware;
use tinyagents_harness::middleware::library::DEFAULT_CLEARED_PLACEHOLDER as CLEARED_PLACEHOLDER;
use tinyagents_harness::middleware::MicrocompactMiddleware;
use tinytools::{ToolRuntime, ToolTimeout};

#[tokio::test]
async fn unavailable_summarization_is_disclosed_in_the_payload() {
    use crate::inference::tokenjuice::module_stub::FAILED_NOTICE;
    let mw = summarizer_mw(StubSummarizer::replying(Err(anyhow::anyhow!(
        "model offline"
    ))));
    let mut ctx = ctx();
    let mut result = tool_result("test_tool", "RAW-TOOL-OUTPUT");

    with_module(mw.after_tool(
        &mut ctx,
        &(),
        &invocation("test-1", "test_tool"),
        &mut result,
    ))
    .await
    .0
    .expect("after_tool should not fail");

    assert!(
        result_text(&result).starts_with(FAILED_NOTICE),
        "the notice must be a PREFIX — the downstream per-tool cap keeps the \
         head, so an appended notice is the first thing truncated away; got: {}",
        result_text(&result)
    );
    assert!(
        result_text(&result).contains("RAW-TOOL-OUTPUT"),
        "disclosure must not cost the payload: {}",
        result_text(&result)
    );
}

#[tokio::test]
async fn a_payload_that_needed_nothing_is_left_completely_alone() {
    // The other half of the contract. If every result carried a notice the
    // marker would be noise and the model would learn to ignore it. Below the
    // summary threshold no call is even prepared.
    let stub = StubSummarizer::replying(Ok("unused".into()));
    let mut mw = summarizer_mw(stub.clone());
    mw.runtime_config = Some(Arc::new(crate::config::Config::default()));
    let mut ctx = ctx();
    let mut result = tool_result("test_tool", "RAW-TOOL-OUTPUT");

    let (outcome, requests) = with_module(mw.after_tool(
        &mut ctx,
        &(),
        &invocation("test-2", "test_tool"),
        &mut result,
    ))
    .await;
    outcome.expect("after_tool should not fail");

    assert_eq!(
        result_text(&result),
        "RAW-TOOL-OUTPUT",
        "a below-threshold payload must be byte-identical"
    );
    assert!(!stub.was_prepared());
    assert!(
        requests.is_empty(),
        "with compaction off and no summary call there is nothing to ask the module"
    );
}

#[tokio::test]
async fn a_summary_model_that_cannot_prepare_leaves_the_payload_intact() {
    // A host misconfiguration (no parent turn to bind to) must never break
    // the tool call; the module is simply not offered a summary call. The
    // payload is untouched, and disclosed as unsummarized like any other
    // failed summary, so the model does not re-run the tool for one.
    struct Unpreparable;
    impl
        tinyagents_harness::summarization::payload_summarizer::PayloadSummarizer<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        > for Unpreparable
    {
        type Prepared = crate::inference::tokenjuice::generate::PreparedGenerate;
        type Error = anyhow::Error;

        fn prepare(
            &self,
            _parent_ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        ) -> anyhow::Result<crate::inference::tokenjuice::generate::PreparedGenerate> {
            Err(anyhow::anyhow!("summarizer misconfigured"))
        }
    }

    let mw = summarizer_mw(Arc::new(Unpreparable));
    let mut ctx = ctx();
    let mut result = tool_result("test_tool", "RAW-TOOL-OUTPUT");

    with_module(mw.after_tool(
        &mut ctx,
        &(),
        &invocation("test-3", "test_tool"),
        &mut result,
    ))
    .await
    .0
    .expect("a summarizer error must never break the tool call");

    let text = result_text(&result);
    assert!(text.starts_with(&crate::inference::tokenjuice::summary_failed_notice()));
    assert!(text.ends_with("RAW-TOOL-OUTPUT"));
}

#[tokio::test]
async fn raw_security_policy_block_is_enriched_with_workaround_and_relay() {
    let mw = outcome_capture_mw();
    let mut result = tool_result(
        "run_command",
        "[policy-blocked] Security policy: read-only mode — only read commands are allowed",
    );
    result = TaToolResult::error(result_text(&result));
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("policy-1", "shell"),
        &mut result,
    )
    .await
    .unwrap();
    // The bare denial now carries a workaround + relay directive, and keeps the
    // marker so classification / the loop-breaker still recognise it.
    assert!(
        result_text(&result).contains("Workaround:"),
        "{}",
        result_text(&result)
    );
    assert!(result_text(&result).contains("Relay this to the user"));
    assert!(result_text(&result).contains(tinytools::status::POLICY_BLOCKED_MARKER));
    assert!(result_text(&result).contains("read-only mode"));
}

#[tokio::test]
async fn already_structured_denial_is_not_double_wrapped() {
    // A ToolPolicyMiddleware-style denial already has "Workaround:"; the capture
    // middleware must leave it untouched (no second Workaround block).
    let mw = outcome_capture_mw();
    let structured =
        "Blocked: Tool 'x' denied. Reason: nope. Workaround: do y. Relay this to the user: ...";
    let mut result = tool_result("x", structured);
    result = TaToolResult::error(result_text(&result));
    mw.after_tool(&mut ctx(), &(), &invocation("policy-2", "x"), &mut result)
        .await
        .unwrap();
    assert_eq!(
        result_text(&result).matches("Workaround:").count(),
        1,
        "must not double-wrap: {}",
        result_text(&result)
    );
}

// ── TurnContextMiddleware config ────────────────────────────────────────

#[test]
fn defaults_enable_the_byte_cap_only() {
    let mw = TurnContextMiddleware::defaults();
    assert_eq!(
        mw.tool_result_budget_bytes,
        tinyagents_harness::config::TurnConfig::default().tool_result_budget_bytes
    );
    assert!(mw.payload_summarizer.is_none());
    assert_eq!(mw.microcompact_keep_recent, 0);
    // Autocompaction defaults on (channel/sub-agent); the chat path overrides
    // it from config.
    assert!(mw.autocompact_enabled);
    // The byte cap alone is enough to make the bundle non-empty (CacheAlign
    // was deleted in C3, so it no longer contributes here).
    assert!(!mw.is_empty());
}

#[test]
fn an_all_default_bundle_installs_nothing() {
    assert!(TurnContextMiddleware::default().is_empty());
}

#[test]
fn tokenjuice_only_bundle_is_not_empty() {
    let mw = TurnContextMiddleware {
        tokenjuice_compaction_enabled: true,
        tokenjuice_compression: AgentTokenjuiceCompression::Light,
        ..Default::default()
    };
    assert!(!mw.is_empty());
}

// ── MicrocompactMiddleware (crate) ──────────────────────────────────────
//
// These assert the crate `MicrocompactMiddleware`, constructed with
// OpenHuman's `CLEARED_PLACEHOLDER`, reproduces the deleted in-house
// middleware byte-for-byte — the parity contract for the upstream swap.

#[tokio::test]
async fn microcompact_clears_older_tool_bodies_and_keeps_recent() {
    let mw = MicrocompactMiddleware::new(1, CLEARED_PLACEHOLDER);
    let mut req = ModelRequest::new(vec![
        TaMessage::system("sys"),
        TaMessage::user("hello"),
        TaMessage::tool("t1", "FIRST_BODY"),
        TaMessage::assistant("thinking"),
        TaMessage::tool("t2", "SECOND_BODY"),
        TaMessage::tool("t3", "THIRD_BODY"),
    ]);

    mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();

    // 3 tool messages, keep_recent=1 → the two oldest cleared, newest kept.
    assert_eq!(req.messages[2].text(), CLEARED_PLACEHOLDER);
    assert_eq!(req.messages[4].text(), CLEARED_PLACEHOLDER);
    assert_eq!(req.messages[5].text(), "THIRD_BODY");
    // Non-tool messages are never touched.
    assert_eq!(req.messages[0].text(), "sys");
    assert_eq!(req.messages[1].text(), "hello");
    assert_eq!(req.messages[3].text(), "thinking");
}

// ── ToolOutputMiddleware ────────────────────────────────────────────────

#[tokio::test]
async fn tool_output_truncates_over_the_flat_budget() {
    let mw = ToolOutputMiddleware {
        budget_bytes: 100,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        tool_policies: HashMap::new(),
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    };
    let mut result = tool_result("echo", &"x".repeat(5_000));
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("echo-capped", "echo"),
        &mut result,
    )
    .await
    .unwrap();
    assert!(
        result_text(&result).len() < 5_000,
        "content should be capped"
    );
    assert!(
        result_text(&result).contains("truncated by tool_result_budget"),
        "a truncation marker should be appended: {}",
        result_text(&result)
    );
}

#[tokio::test]
async fn tool_output_leaves_small_results_untouched() {
    let mw = ToolOutputMiddleware {
        budget_bytes: 1_000,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        tool_policies: HashMap::new(),
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    };
    let mut result = tool_result("echo", "tiny");
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("echo-small", "echo"),
        &mut result,
    )
    .await
    .unwrap();
    assert_eq!(result_text(&result), "tiny");
}

#[test]
fn tool_char_cap_reads_the_tools_own_declared_cap() {
    let mut tool_policies = HashMap::new();
    tool_policies.insert(
        "big".to_string(),
        TaToolPolicy::classified().with_runtime(ToolRuntime {
            timeout_ms: None,
            timeout: ToolTimeout::Inherit,
            max_retries: None,
            idempotent: false,
            cancelable: true,
            sandbox: tinytools::SandboxMode::Inherit,
            max_result_bytes: Some(10),
            streaming: false,
            replay: Default::default(),
        }),
    );
    let mw = ToolOutputMiddleware {
        budget_bytes: 1_000,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        tool_policies,
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    };
    // Tool declares its own char cap → surfaced for the per-tool truncation.
    assert_eq!(mw.tool_char_cap("big"), Some(10));
    // Unknown tool → no per-tool cap (the flat byte budget applies instead).
    assert_eq!(mw.tool_char_cap("other"), None);
}

/// openhuman#5722 review: the disclosure used to be prefixed *before* the
/// per-tool char cap, so a tool declaring a cap shorter than the notice had
/// `chars().take(cap)` slice through the notice itself — dropping the
/// reason and the do-not-re-run sentence, and leaving the model a truncated
/// fragment that still reads as tool output. The notice is applied after
/// every cap now, so it survives intact whatever the tool declared.
#[tokio::test]
async fn a_tool_that_caps_itself_is_never_sent_to_the_summarizer() {
    // The cost bug this replaced. The per-tool cap used to be applied
    // *after* the summarizer, so a tool declaring `max_result_size_chars`
    // still shipped its full body to an LLM and the cap only bounded the
    // summary. One research turn paid 1,083,069 input tokens that way.
    // A tool that caps itself is already bounded, and step 4 spills the
    // remainder to a pageable artifact, so the model call buys nothing.
    let mut tool_policies = HashMap::new();
    tool_policies.insert(
        "terse".to_string(),
        TaToolPolicy::classified().with_runtime(ToolRuntime {
            timeout_ms: None,
            timeout: ToolTimeout::Inherit,
            max_retries: None,
            idempotent: false,
            cancelable: true,
            sandbox: tinytools::SandboxMode::Inherit,
            max_result_bytes: Some(64),
            streaming: false,
            replay: Default::default(),
        }),
    );
    let stub = StubSummarizer::replying(Ok("note".into()));
    let mut mw = summarizer_mw(stub.clone());
    mw.tool_policies = tool_policies;

    let mut result = tool_result("terse", &"payload ".repeat(200));
    let (outcome, requests) = with_module(mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("terse-notice", "terse"),
        &mut result,
    ))
    .await;
    outcome.unwrap();

    assert!(
        !stub.was_prepared() && requests.is_empty(),
        "a tool with its own cap must not be dispatched to the summarizer"
    );
    assert!(
        result_text(&result).len() < 1_600,
        "the cap must still bound the result: {} bytes",
        result_text(&result).len()
    );
}

/// The exception to the rule above: a caller that said what it needs from a
/// self-capped tool (`web_fetch` with `summary_focus`) gets a summary written
/// for that, because paging the raw page cannot answer the question.
#[tokio::test]
async fn a_tool_that_caps_itself_is_summarized_when_the_caller_gives_a_focus() {
    let mut tool_policies = HashMap::new();
    tool_policies.insert(
        "terse".to_string(),
        TaToolPolicy::classified().with_runtime(ToolRuntime {
            timeout_ms: None,
            timeout: ToolTimeout::Inherit,
            max_retries: None,
            idempotent: false,
            cancelable: true,
            sandbox: tinytools::SandboxMode::Inherit,
            max_result_bytes: Some(64),
            streaming: false,
            replay: Default::default(),
        }),
    );
    let stub = StubSummarizer::replying(Ok("focused".into()));
    let mut mw = summarizer_mw(stub.clone());
    mw.tool_policies = tool_policies;
    mw.summary_focus_tools = ["terse".to_string()].into();
    let mut call = TaToolCall::new("terse-focus", "terse", json!({"summary_focus": "errors"}));
    let mut ctx = ctx();
    mw.before_tool(&mut ctx, &(), &mut call).await.unwrap();

    let mut result = tool_result("terse", &"payload ".repeat(200));
    let (outcome, requests) = with_module(mw.after_tool(
        &mut ctx,
        &(),
        &invocation("terse-focus", "terse"),
        &mut result,
    ))
    .await;
    outcome.unwrap();

    assert!(stub.was_prepared());
    assert_eq!(requests[0].focus.as_deref(), Some("errors"));
    assert!(result_text(&result).contains("focused"));
}

#[tokio::test]
async fn a_raw_web_fetch_never_prepares_a_payload_summary() {
    let stub = StubSummarizer::replying(Ok("must remain unused".into()));
    let mw = summarizer_mw(stub.clone());
    let mut call = TaToolCall::new(
        "raw-fetch",
        "web_fetch",
        json!({"url": "https://example.test", "raw": true}),
    );
    let mut ctx = ctx();
    mw.before_tool(&mut ctx, &(), &mut call)
        .await
        .expect("raw fetch is recorded before execution");

    let mut result = tool_result("web_fetch", &"<html>markup</html>".repeat(300));
    let (outcome, requests) = with_module(mw.after_tool(
        &mut ctx,
        &(),
        &invocation("raw-fetch", "web_fetch"),
        &mut result,
    ))
    .await;

    outcome.expect("raw fetch result is processed");
    assert!(
        !stub.was_prepared() && requests.is_empty(),
        "raw fetches must bypass the payload summarizer and TinyJuice"
    );
}

#[tokio::test]
async fn tool_output_honors_a_tools_own_cap() {
    let mut tool_policies = HashMap::new();
    tool_policies.insert(
        "capped".to_string(),
        TaToolPolicy::classified().with_runtime(ToolRuntime {
            timeout_ms: None,
            timeout: ToolTimeout::Inherit,
            max_retries: None,
            idempotent: false,
            cancelable: true,
            sandbox: tinytools::SandboxMode::Inherit,
            max_result_bytes: Some(20),
            streaming: false,
            replay: Default::default(),
        }),
    );
    let mw = ToolOutputMiddleware {
        budget_bytes: 100_000,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        tool_policies,
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    };
    let mut result = tool_result("capped", &"y".repeat(500));
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("capped", "capped"),
        &mut result,
    )
    .await
    .unwrap();
    let text = result_text(&result);
    assert!(
        text.len() < 500,
        "the tool's own 20-byte cap must bound the result: {text}"
    );
    assert!(
        text.contains("truncated by tool_result_budget"),
        "a capped tool now takes the shared spill path, which says how much \
         is missing and how to get it: {text}"
    );
}

/// The shared tool-result byte backstop is a user-visible truncation limit;
/// moving the ladder into the harness must not change it.
#[test]
fn shared_tool_result_byte_budget_stays_16_kib() {
    assert_eq!(
        tinyagents_harness::config::TurnConfig::default().tool_result_budget_bytes,
        16 * 1024
    );
}

#[path = "middleware_tool_output_compaction_tests.rs"]
mod compaction_tests;
