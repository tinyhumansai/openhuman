use super::*;
use crate::openhuman::agent::harness::definition::{
    AgentDefinition, DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
};

fn dummy_definition() -> AgentDefinition {
    AgentDefinition {
        id: "summarizer".into(),
        when_to_use: "test".into(),
        display_name: Some("Summarizer".into()),
        system_prompt: PromptSource::Inline("test prompt".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Hint("summarization".into()),
        temperature: 0.2,
        tools: ToolScope::Named(vec![]),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 1,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

// Tests use the production-default thresholds expressed as tokens:
// 500 000 tokens lower bound, 2 000 000 tokens upper bound.
// Since estimate_tokens = chars / 4, 1 char ≈ 0.25 tokens.
const TEST_THRESHOLD_TOKENS: usize = 500_000;
const TEST_MAX_TOKENS: usize = 2_000_000;

fn dummy_parent_ctx() -> RunContext<()> {
    RunContext::new(tinyagents_harness::context::RunConfig::new("test"), ())
}

#[tokio::test]
async fn below_threshold_is_not_needed_not_unavailable() {
    let summarizer =
        SubagentPayloadSummarizer::new(dummy_definition(), TEST_THRESHOLD_TOKENS, TEST_MAX_TOKENS);
    // 1 KB of 'x' → ~256 tokens, well below the 500 000 threshold.
    let raw = "x".repeat(1_024);
    let outcome = summarizer
        .maybe_summarize_in_parent(&dummy_parent_ctx(), "test_tool", None, &raw)
        .await
        .expect("below-threshold check should not error");
    assert!(
        matches!(outcome, SummarizeOutcome::NotNeeded),
        "a payload below the threshold needed nothing, so the model must be \
         told nothing; got {outcome:?}"
    );
}

#[tokio::test]
async fn above_max_cap_is_disclosed_as_unavailable() {
    let summarizer =
        SubagentPayloadSummarizer::new(dummy_definition(), TEST_THRESHOLD_TOKENS, TEST_MAX_TOKENS);
    // 9 MB of 'x' → ~2 359 296 tokens, above the 2 000 000 cap.
    let raw = "x".repeat(9 * 1024 * 1024);
    let outcome = summarizer
        .maybe_summarize_in_parent(&dummy_parent_ctx(), "test_tool", None, &raw)
        .await
        .expect("above-cap check should not error");
    // The regression guard. This used to be `outcome.is_none()`, the same
    // value the below-threshold case returns — which is exactly how a
    // payload that will be truncated downstream reached the model looking
    // like ordinary output.
    assert!(
        matches!(
            outcome,
            SummarizeOutcome::Unavailable(UnavailableReason::PayloadTooLarge)
        ),
        "a payload over the cap is not summarized and will be truncated, so \
         it must be disclosed, not passed through silently; got {outcome:?}"
    );
}

#[tokio::test]
async fn tripped_breaker_is_disclosed_as_unavailable() {
    let summarizer =
        SubagentPayloadSummarizer::new(dummy_definition(), TEST_THRESHOLD_TOKENS, TEST_MAX_TOKENS);
    // Manually trip the breaker by recording 3 failures.
    summarizer.record_failure();
    summarizer.record_failure();
    summarizer.record_failure();
    assert!(summarizer.breaker_tripped(), "breaker should be tripped");

    // 3 MB of 'x' → ~786 432 tokens: inside the [500k, 2M] summarize
    // window, so would normally dispatch — but breaker is tripped.
    let raw = "x".repeat(3 * 1024 * 1024);
    let outcome = summarizer
        .maybe_summarize_in_parent(&dummy_parent_ctx(), "test_tool", None, &raw)
        .await
        .expect("breaker check should not error");
    assert!(
        matches!(
            outcome,
            SummarizeOutcome::Unavailable(UnavailableReason::Disabled)
        ),
        "a tripped breaker must short-circuit before any dispatch AND be \
         disclosed — it is the case that repeats most often, because the \
         breaker is rebuilt per session build; got {outcome:?}"
    );
}

#[test]
fn every_unavailable_notice_says_not_to_re_run_without_predicting_the_future() {
    // The whole point of the notice. A model handed a truncated dump with
    // no explanation does the reasonable thing and calls the same tool
    // again, which is the re-dispatch loop a user perceives as a hang.
    //
    // The second half of the name is the part that took two rounds to get
    // right. The instruction has to hold for every variant *without*
    // asserting what a re-run would do, because `Failed` is recorded before
    // the breaker opens and a later attempt can genuinely succeed — so a
    // notice claiming otherwise contradicts the breaker. Both previously
    // shipped phrasings are asserted absent below so neither comes back as
    // a tightening.
    for reason in [
        UnavailableReason::PayloadTooLarge,
        UnavailableReason::Disabled,
        UnavailableReason::Failed,
    ] {
        let notice = reason.notice();
        assert!(
            notice.contains("Do not re-run the tool for a summary"),
            "{reason:?} must tell the model not to retry: {notice}"
        );
        for prediction in ["will return the same result", "will not produce a summary"] {
            assert!(
                !notice.contains(prediction),
                "{reason:?} must not predict what a re-run would do ({prediction:?}): \
                 {notice}"
            );
        }
        assert!(
            notice.starts_with("[openhuman: summarization unavailable"),
            "{reason:?} must be greppable and self-identifying: {notice}"
        );
    }
}

#[test]
fn the_three_notices_are_distinct() {
    // Three different situations; a reader (human or model) should be able
    // to tell which one happened.
    let all = [
        UnavailableReason::PayloadTooLarge.notice(),
        UnavailableReason::Disabled.notice(),
        UnavailableReason::Failed.notice(),
    ];
    for (i, a) in all.iter().enumerate() {
        for b in all.iter().skip(i + 1) {
            assert_ne!(a, b, "each reason needs its own notice");
        }
    }
}

#[test]
fn build_summarizer_prompt_includes_tool_name_and_hint() {
    let prompt = build_summarizer_prompt(
        "GITHUB_LIST_REPOSITORY_ISSUES",
        Some("find the most urgent open issues"),
        "{\"issues\": [{\"id\": 1}]}",
    );
    assert!(prompt.contains("GITHUB_LIST_REPOSITORY_ISSUES"));
    assert!(prompt.contains("find the most urgent open issues"));
    assert!(prompt.contains("Parent task hint:"));
    assert!(prompt.contains("--- BEGIN ---"));
    assert!(prompt.contains("--- END ---"));
    assert!(prompt.contains("{\"issues\": [{\"id\": 1}]}"));
}

#[test]
fn build_summarizer_prompt_omits_hint_when_none() {
    let prompt = build_summarizer_prompt("file_read", None, "log line 1\nlog line 2");
    assert!(prompt.contains("file_read"));
    assert!(prompt.contains("--- BEGIN ---"));
    assert!(prompt.contains("--- END ---"));
    assert!(prompt.contains("log line 1"));
    assert!(
        !prompt.contains("Parent task hint:"),
        "no hint line should be present when hint is None"
    );
}

#[test]
fn record_success_resets_breaker() {
    let summarizer =
        SubagentPayloadSummarizer::new(dummy_definition(), TEST_THRESHOLD_TOKENS, TEST_MAX_TOKENS);
    summarizer.record_failure();
    summarizer.record_failure();
    assert!(!summarizer.breaker_tripped());
    summarizer.record_success();
    // Even one more failure now should not trip — counter was reset.
    summarizer.record_failure();
    assert!(!summarizer.breaker_tripped());
}
