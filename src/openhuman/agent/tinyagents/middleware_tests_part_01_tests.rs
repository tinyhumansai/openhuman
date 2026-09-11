// Imported here rather than in `middleware_part_01.rs` since #6014: the
// production install site moved to `assemble_turn_harness` (which names it
// fully-qualified), so the module itself no longer references the type.
use super::*;
use tinyagents_harness::middleware::MicrocompactMiddleware;

// #4462: image-aware token estimation. A base64 image marker must be priced
// at the flat IMAGE_MARKER_TOKEN_COST, not chars/4 of its payload — otherwise
// one image reads as millions of tokens and the trimmer evicts everything,
// including the system prompt.

#[test]
fn estimate_text_tokens_markerless_is_chars_over_four() {
    assert_eq!(estimate_text_tokens(&"a".repeat(40)), (40 + 3) / 4);
    assert_eq!(estimate_text_tokens(""), 0);
}

#[test]
fn estimate_text_tokens_prices_image_marker_flat_not_by_length() {
    let huge = "x".repeat(40_000);
    let text = format!("[IMAGE:{huge}]");
    let tokens = estimate_text_tokens(&text);
    // chars/4 of the payload would be ~10_000; the flat price is 1_200.
    assert!(
        tokens >= IMAGE_MARKER_TOKEN_COST,
        "at least the flat image cost: {tokens}"
    );
    assert!(
        tokens < 2_000,
        "image priced flat, not by base64 length: {tokens}"
    );
}

#[test]
fn estimate_text_tokens_charges_each_image_marker_once() {
    let tokens = estimate_text_tokens("[IMAGE:aaaa] and [IMAGE:bbbb]");
    assert!(
        tokens >= 2 * IMAGE_MARKER_TOKEN_COST,
        "two images each priced: {tokens}"
    );
    assert!(
        tokens < 2 * IMAGE_MARKER_TOKEN_COST + 100,
        "no runaway from the surrounding text: {tokens}"
    );
}

#[tokio::test]
async fn unavailable_summarization_is_disclosed_in_the_payload() {
    let mw = summarizer_mw(StubSummarizer::ok(SummarizeOutcome::Unavailable(
        UnavailableReason::Failed,
    )));
    let mut ctx = ctx();
    let mut result = tool_result("test_tool", "RAW-TOOL-OUTPUT");

    mw.after_tool(&mut ctx, &(), &mut result)
        .await
        .expect("after_tool should not fail");

    assert!(
        result
            .content
            .starts_with(UnavailableReason::Failed.notice()),
        "the notice must be a PREFIX — the downstream per-tool cap keeps the \
         head, so an appended notice is the first thing truncated away; got: {}",
        result.content
    );
    assert!(
        result.content.contains("RAW-TOOL-OUTPUT"),
        "disclosure must not cost the payload: {}",
        result.content
    );
}

#[tokio::test]
async fn a_payload_that_needed_nothing_is_left_completely_alone() {
    // The other half of the contract. If every result carried a notice the
    // marker would be noise and the model would learn to ignore it.
    let mw = summarizer_mw(StubSummarizer::ok(SummarizeOutcome::NotNeeded));
    let mut ctx = ctx();
    let mut result = tool_result("test_tool", "RAW-TOOL-OUTPUT");

    mw.after_tool(&mut ctx, &(), &mut result)
        .await
        .expect("after_tool should not fail");

    assert_eq!(
        result.content, "RAW-TOOL-OUTPUT",
        "a below-threshold payload must be byte-identical"
    );
}

#[tokio::test]
async fn a_summarizer_error_is_disclosed_rather_than_swallowed() {
    // `Err` used to be discarded by the same `if let Ok(Some(..))` that
    // discarded `None`, so a fatal misconfiguration was indistinguishable
    // from "nothing to do".
    struct ErroringSummarizer;
    #[async_trait]
    impl PayloadSummarizer for ErroringSummarizer {
        async fn maybe_summarize_in_parent(
            &self,
            _parent_ctx: &RunContext<()>,
            _tool_name: &str,
            _parent_task_hint: Option<&str>,
            _raw: &str,
        ) -> anyhow::Result<SummarizeOutcome> {
            Err(anyhow::anyhow!("summarizer misconfigured"))
        }
    }

    let mw = summarizer_mw(Arc::new(ErroringSummarizer));
    let mut ctx = ctx();
    let mut result = tool_result("test_tool", "RAW-TOOL-OUTPUT");

    mw.after_tool(&mut ctx, &(), &mut result)
        .await
        .expect("a summarizer error must never break the tool call");

    assert!(
        result
            .content
            .starts_with(UnavailableReason::Failed.notice()),
        "an errored summarizer must be disclosed too; got: {}",
        result.content
    );
}

#[tokio::test]
async fn prompt_cache_segments_fingerprint_full_tool_schema() {
    let mw = PromptCacheSegmentMiddleware;
    let mut first =
        ModelRequest::new(vec![TaMessage::system("sys")]).with_tools(vec![ToolSchema::new(
            "lookup",
            "lookup a user",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
                },
            }),
        )]);
    mw.before_model(&mut ctx(), &(), &mut first).await.unwrap();

    let mut second =
        ModelRequest::new(vec![TaMessage::system("sys")]).with_tools(vec![ToolSchema::new(
            "lookup",
            "lookup a user",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" }
                },
            }),
        )]);
    mw.before_model(&mut ctx(), &(), &mut second).await.unwrap();

    let first_tool_segment = first
        .cache_segments
        .iter()
        .find(|segment| segment.role == SegmentRole::Tools)
        .expect("tool segment");
    let second_tool_segment = second
        .cache_segments
        .iter()
        .find(|segment| segment.role == SegmentRole::Tools)
        .expect("tool segment");

    assert_ne!(
        first_tool_segment.id, second_tool_segment.id,
        "same-name tools with different schemas must bust the stable prefix"
    );
    assert_ne!(first.prompt_fingerprint, second.prompt_fingerprint);
    assert_eq!(
        first.prompt_fingerprint.as_deref().unwrap().len(),
        64,
        "request prompt fingerprints use TinyAgents' SHA-256 shape"
    );
}

#[tokio::test]
async fn raw_security_policy_block_is_enriched_with_workaround_and_relay() {
    let mw = outcome_capture_mw();
    let mut result = tool_result(
        "run_command",
        "[policy-blocked] Security policy: read-only mode — only read commands are allowed",
    );
    result.error = Some(result.content.clone());
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    // The bare denial now carries a workaround + relay directive, and keeps the
    // marker so classification / the loop-breaker still recognise it.
    assert!(result.content.contains("Workaround:"), "{}", result.content);
    assert!(result.content.contains("Relay this to the user"));
    assert!(result
        .content
        .contains(crate::openhuman::security::POLICY_BLOCKED_MARKER));
    assert!(result.content.contains("read-only mode"));
}

#[tokio::test]
async fn already_structured_denial_is_not_double_wrapped() {
    // A ToolPolicyMiddleware-style denial already has "Workaround:"; the capture
    // middleware must leave it untouched (no second Workaround block).
    let mw = outcome_capture_mw();
    let structured =
        "Blocked: Tool 'x' denied. Reason: nope. Workaround: do y. Relay this to the user: ...";
    let mut result = tool_result("x", structured);
    result.error = Some(result.content.clone());
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_eq!(
        result.content.matches("Workaround:").count(),
        1,
        "must not double-wrap: {}",
        result.content
    );
}

// ── TurnContextMiddleware config ────────────────────────────────────────

#[test]
fn defaults_enable_the_byte_cap_only() {
    let mw = TurnContextMiddleware::defaults();
    assert_eq!(
        mw.tool_result_budget_bytes,
        DEFAULT_TOOL_RESULT_BUDGET_BYTES
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

#[tokio::test]
async fn microcompact_is_a_noop_when_within_keep_recent() {
    let mw = MicrocompactMiddleware::new(5, CLEARED_PLACEHOLDER);
    let mut req = ModelRequest::new(vec![TaMessage::tool("t1", "A"), TaMessage::tool("t2", "B")]);
    mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();
    assert_eq!(req.messages[0].text(), "A");
    assert_eq!(req.messages[1].text(), "B");
}

#[tokio::test]
async fn microcompact_is_idempotent() {
    let mw = MicrocompactMiddleware::new(1, CLEARED_PLACEHOLDER);
    let mut req = ModelRequest::new(vec![
        TaMessage::tool("t1", "FIRST"),
        TaMessage::tool("t2", "SECOND"),
    ]);
    mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();
    let after_first = req.messages[0].text();
    assert_eq!(after_first, CLEARED_PLACEHOLDER);
    // Second pass leaves the already-cleared body as the placeholder.
    mw.before_model(&mut ctx(), &(), &mut req).await.unwrap();
    assert_eq!(req.messages[0].text(), CLEARED_PLACEHOLDER);
    assert_eq!(req.messages[1].text(), "SECOND");
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
        tool_policies: HashMap::new(),
    };
    let mut result = tool_result("echo", &"x".repeat(5_000));
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert!(result.content.len() < 5_000, "content should be capped");
    assert!(
        result.content.contains("truncated by tool_result_budget"),
        "a truncation marker should be appended: {}",
        result.content
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
        tool_policies: HashMap::new(),
    };
    let mut result = tool_result("echo", "tiny");
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_eq!(result.content, "tiny");
}

#[test]
fn tool_char_cap_reads_the_tools_own_declared_cap() {
    let mut tool_policies = HashMap::new();
    tool_policies.insert(
        "big".to_string(),
        TaToolPolicy::classified().with_runtime(tinyagents_harness::tool::ToolRuntime {
            timeout_ms: None,
            timeout: tinyagents_harness::tool::ToolTimeout::Inherit,
            max_retries: None,
            idempotent: false,
            cancelable: true,
            sandbox: tinyagents_harness::tool::SandboxMode::Inherit,
            max_result_bytes: Some(10),
            streaming: false,
        }),
    );
    let mw = ToolOutputMiddleware {
        budget_bytes: 1_000,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        tool_policies,
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
async fn an_unavailable_notice_survives_a_tool_cap_shorter_than_itself() {
    let mut tool_policies = HashMap::new();
    tool_policies.insert(
        "terse".to_string(),
        TaToolPolicy::classified().with_runtime(tinyagents_harness::tool::ToolRuntime {
            timeout_ms: None,
            timeout: tinyagents_harness::tool::ToolTimeout::Inherit,
            max_retries: None,
            idempotent: false,
            cancelable: true,
            sandbox: tinyagents_harness::tool::SandboxMode::Inherit,
            // Far shorter than the ~165-char notice.
            max_result_bytes: Some(12),
            streaming: false,
        }),
    );
    let mw = ToolOutputMiddleware {
        // Large enough that the byte-budget backstop never fires, so this
        // observes the per-tool cap alone.
        budget_bytes: 10_000_000,
        payload_summarizer: Some(StubSummarizer::ok(SummarizeOutcome::Unavailable(
            UnavailableReason::Failed,
        ))),
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Off,
        tool_policies,
    };

    let mut result = tool_result("terse", &"payload ".repeat(200));
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();

    let notice = UnavailableReason::Failed.notice();
    assert!(
        result.content.starts_with(notice),
        "the complete notice must lead the content, got {:?}",
        result.content.chars().take(200).collect::<String>()
    );
    assert!(
        result
            .content
            .contains("Do not re-run the tool for a summary"),
        "the do-not-re-run instruction is the whole point of the notice and must survive"
    );
    // The payload itself is still capped — deferring the notice must not
    // smuggle the tool past its own declared limit.
    let payload = result
        .content
        .strip_prefix(notice)
        .expect("notice prefix")
        .trim_start();
    assert!(
        payload.contains("[truncated by tool cap:"),
        "the raw payload must still be truncated to the tool's cap, got {payload:?}"
    );
}

#[tokio::test]
async fn tool_output_honors_a_tools_own_cap() {
    let mut tool_policies = HashMap::new();
    tool_policies.insert(
        "capped".to_string(),
        TaToolPolicy::classified().with_runtime(tinyagents_harness::tool::ToolRuntime {
            timeout_ms: None,
            timeout: tinyagents_harness::tool::ToolTimeout::Inherit,
            max_retries: None,
            idempotent: false,
            cancelable: true,
            sandbox: tinyagents_harness::tool::SandboxMode::Inherit,
            max_result_bytes: Some(20),
            streaming: false,
        }),
    );
    let mw = ToolOutputMiddleware {
        budget_bytes: 100_000,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        tool_policies,
    };
    let mut result = tool_result("capped", &"y".repeat(500));
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert!(
        result
            .content
            .contains("truncated by tool cap: 480 more chars not shown"),
        "the tool's own 20-char cap should truncate with the tool-cap marker: {}",
        result.content
    );
}

#[test]
fn compaction_exempt_tools_contains_every_proposal_tool() {
    for tool in [
        "propose_workflow",
        "revise_workflow",
        "edit_workflow",
        "save_workflow",
        "create_workflow",
    ] {
        assert!(
            COMPACTION_EXEMPT_TOOLS.contains(&tool),
            "{tool} must be exempt from tokenjuice/summarizer compaction"
        );
    }
}

#[tokio::test]
#[ignore = "requires a built TinyJuice module"]
async fn tool_output_tabulates_a_large_graph_for_a_non_exempt_tool() {
    // Sanity baseline proving this test's payload actually exercises real
    // tinyjuice tabulation (and isn't just below-threshold): a tool name
    // NOT in COMPACTION_EXEMPT_TOOLS loses the `"type"` marker.
    // Resolve the explicit release fixture before `after_tool` performs
    // ambient config initialisation. A pristine CI workspace otherwise
    // exercises the production fail-open path before the test override is
    // admitted, hiding a usable module behind unchanged output.
    crate::openhuman::inference::tokenjuice::install_from_config(
        &crate::openhuman::config::Config::default(),
    )
    .await
    .expect("released TinyJuice module must load and accept host configuration");
    let mw = compaction_enabled_mw();
    let payload = large_workflow_proposal_json();
    assert!(
        payload.len()
            >= crate::openhuman::config::Config::default()
                .tokenjuice
                .min_bytes_to_compress,
        "baseline payload must clear OpenHuman's configured compaction floor"
    );
    let mut result = tool_result("some_other_tool", &payload);
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_ne!(
        result.content, payload,
        "a non-exempt tool's large uniform-array payload should be rewritten by tokenjuice"
    );
    let reparsed: Result<serde_json::Value, _> = serde_json::from_str(&result.content);
    let marker_survived = reparsed
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str().map(str::to_string)))
        == Some("workflow_proposal".to_string());
    assert!(
        !marker_survived,
        "baseline expectation: tabulation strips the type marker for non-exempt tools"
    );
}

#[tokio::test]
async fn tool_output_leaves_propose_workflow_byte_for_byte_intact() {
    let mw = compaction_enabled_mw();
    let payload = large_workflow_proposal_json();
    let mut result = tool_result("propose_workflow", &payload);
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_eq!(
        result.content, payload,
        "propose_workflow results must pass through compaction untouched"
    );
    let reparsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(reparsed["type"], "workflow_proposal");
    assert_eq!(reparsed["graph"]["nodes"].as_array().unwrap().len(), 20);
}

#[tokio::test]
async fn tool_output_leaves_every_exempt_tool_name_intact() {
    let mw = compaction_enabled_mw();
    let payload = large_workflow_proposal_json();
    for tool in COMPACTION_EXEMPT_TOOLS {
        let mut result = tool_result(tool, &payload);
        mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
        assert_eq!(
            result.content, payload,
            "{tool}'s result must pass through compaction untouched"
        );
    }
}

#[tokio::test]
async fn tool_output_leaves_an_oversized_propose_workflow_byte_for_byte_intact() {
    // Gap 1: a ≥10-node proposal routinely exceeds the ~16 KiB shared
    // byte-budget backstop. Before the truncation exemption, step 4
    // truncated it at a UTF-8 boundary — invalid JSON, so both
    // `flows::ops::extract_workflow_proposal` and the frontend's
    // `parseWorkflowProposal` silently fell back to `proposal: None` and a
    // blank canvas. This must survive byte-for-byte regardless of size.
    let mw = truncation_probe_mw();
    let payload = oversized_workflow_proposal_json(30);
    assert!(
        payload.len() > DEFAULT_TOOL_RESULT_BUDGET_BYTES,
        "test payload must exceed the shared byte budget to exercise step 4: {} bytes",
        payload.len()
    );
    let mut result = tool_result("propose_workflow", &payload);
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_eq!(
        result.content, payload,
        "an oversized propose_workflow result must not be truncated by the shared byte-budget backstop"
    );
    let reparsed: serde_json::Value = serde_json::from_str(&result.content)
        .expect("must still be valid JSON after passing through after_tool");
    assert_eq!(reparsed["type"], "workflow_proposal");
    assert_eq!(reparsed["graph"]["nodes"].as_array().unwrap().len(), 30);
}

#[tokio::test]
async fn tool_output_truncates_the_same_oversized_payload_for_a_non_exempt_tool() {
    // Baseline pairing with the test above: proves the identical oversized
    // payload IS truncated (and consequently unparseable) for a tool that
    // is NOT truncation-exempt, so the exemption test isn't vacuously true
    // because the payload never actually crossed the budget.
    let mw = truncation_probe_mw();
    let payload = oversized_workflow_proposal_json(30);
    let mut result = tool_result("some_other_tool", &payload);
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_ne!(
        result.content, payload,
        "a non-exempt tool's oversized payload should be truncated by the shared byte-budget backstop"
    );
    assert!(
        result.content.contains("truncated by tool_result_budget"),
        "expected the byte-budget truncation marker: {}",
        result.content
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&result.content).is_err(),
        "truncated JSON should no longer parse as a whole document"
    );
}

#[tokio::test]
async fn get_tool_output_sample_is_compaction_exempt() {
    // Gap 2: tokenjuice tabulation elides the very array the model calls
    // this tool to observe, so it would derive a wrong or nonexistent
    // `split_out.path` from the tabulated summary instead of the real
    // response shape. The sample must reach the model untabulated.
    let mw = compaction_enabled_mw();
    let payload = large_sample_response_json(10);
    let mut result = tool_result("get_tool_output_sample", &payload);
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_eq!(
        result.content, payload,
        "get_tool_output_sample's response must not be tokenjuice-tabulated"
    );
}

#[tokio::test]
async fn get_tool_contract_is_compaction_exempt() {
    let mw = compaction_enabled_mw();
    let payload = large_sample_response_json(10);
    let mut result = tool_result("get_tool_contract", &payload);
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_eq!(
        result.content, payload,
        "get_tool_contract's response must not be tokenjuice-tabulated"
    );
}
