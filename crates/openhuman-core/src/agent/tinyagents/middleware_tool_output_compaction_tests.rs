use super::*;
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
async fn tool_output_tabulates_a_large_graph_for_a_non_exempt_tool() {
    // Sanity baseline proving this test's payload actually exercises real
    // tinyjuice tabulation (and isn't just below-threshold): a tool name
    // NOT in COMPACTION_EXEMPT_TOOLS loses the `"type"` marker.
    // Resolve the explicit release fixture before `after_tool` performs
    // ambient config initialisation. A pristine CI workspace otherwise
    // exercises the production fail-open path before the test override is
    // admitted, hiding a usable module behind unchanged output.
    crate::inference::tokenjuice::install_from_config(&crate::config::Config::default())
        .await
        .expect("released TinyJuice module must load and accept host configuration");
    let mw = compaction_enabled_mw();
    let payload = large_workflow_proposal_json();
    assert!(
        payload.len()
            >= crate::config::Config::default()
                .tokenjuice
                .min_bytes_to_compress,
        "baseline payload must clear OpenHuman's configured compaction floor"
    );
    let mut result = tool_result("some_other_tool", &payload);
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("other-compact", "some_other_tool"),
        &mut result,
    )
    .await
    .unwrap();
    assert_ne!(
        result_text(&result),
        payload,
        "a non-exempt tool's large uniform-array payload should be rewritten by tokenjuice"
    );
    let reparsed: Result<serde_json::Value, _> = serde_json::from_str(&result_text(&result));
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
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("proposal-compact", "propose_workflow"),
        &mut result,
    )
    .await
    .unwrap();
    assert_eq!(
        result_text(&result),
        payload,
        "propose_workflow results must pass through compaction untouched"
    );
    let reparsed: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
    assert_eq!(reparsed["type"], "workflow_proposal");
    assert_eq!(reparsed["graph"]["nodes"].as_array().unwrap().len(), 20);
}

#[tokio::test]
async fn tool_output_leaves_every_exempt_tool_name_intact() {
    let mw = compaction_enabled_mw();
    let payload = large_workflow_proposal_json();
    for tool in COMPACTION_EXEMPT_TOOLS {
        let mut result = tool_result(tool, &payload);
        mw.after_tool(
            &mut ctx(),
            &(),
            &invocation(format!("exempt-{tool}"), *tool),
            &mut result,
        )
        .await
        .unwrap();
        assert_eq!(
            result_text(&result),
            payload,
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
        payload.len() > tinyagents_harness::config::TurnConfig::default().tool_result_budget_bytes,
        "test payload must exceed the shared byte budget to exercise step 4: {} bytes",
        payload.len()
    );
    let mut result = tool_result("propose_workflow", &payload);
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("proposal-truncate", "propose_workflow"),
        &mut result,
    )
    .await
    .unwrap();
    assert_eq!(
        result_text(&result),
        payload,
        "an oversized propose_workflow result must not be truncated by the shared byte-budget backstop"
    );
    let reparsed: serde_json::Value = serde_json::from_str(&result_text(&result))
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
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("other-truncate", "some_other_tool"),
        &mut result,
    )
    .await
    .unwrap();
    assert_ne!(
        result_text(&result),
        payload,
        "a non-exempt tool's oversized payload should be truncated by the shared byte-budget backstop"
    );
    assert!(
        result_text(&result).contains("truncated by tool_result_budget"),
        "expected the byte-budget truncation marker: {}",
        result_text(&result)
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&result_text(&result)).is_err(),
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
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("sample", "get_tool_output_sample"),
        &mut result,
    )
    .await
    .unwrap();
    assert_eq!(
        result_text(&result),
        payload,
        "get_tool_output_sample's response must not be tokenjuice-tabulated"
    );
}

#[tokio::test]
async fn get_tool_contract_is_compaction_exempt() {
    let mw = compaction_enabled_mw();
    let payload = large_sample_response_json(10);
    let mut result = tool_result("get_tool_contract", &payload);
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("contract", "get_tool_contract"),
        &mut result,
    )
    .await
    .unwrap();
    assert_eq!(
        result_text(&result),
        payload,
        "get_tool_contract's response must not be tokenjuice-tabulated"
    );
}

/// The caller's `summary_focus` is an argument to the summary of the result,
/// not to the tool: `before_tool` takes it out of the call, and it reaches
/// TinyJuice with the result together with a summary call bound to the turn.
#[tokio::test]
async fn the_callers_focus_reaches_tinyjuice_and_never_the_tool() {
    let mw = summarizer_mw(StubSummarizer::replying(Ok("focused note".into())));
    let mut call = TaToolCall::new(
        "fetch-1",
        "web_fetch",
        json!({"url": "https://example.com/docs", "summary_focus": "the rate limits"}),
    );
    let mut ctx = ctx();
    mw.before_tool(&mut ctx, &(), &mut call).await.unwrap();
    assert_eq!(
        call.arguments,
        json!({"url": "https://example.com/docs"}),
        "the tool must never see summary_focus"
    );

    let mut result = tool_result("web_fetch", &"page text ".repeat(200));
    let (outcome, requests) = with_module(mw.after_tool(
        &mut ctx,
        &(),
        &invocation("fetch-1", "web_fetch"),
        &mut result,
    ))
    .await;
    outcome.unwrap();

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].focus.as_deref(), Some("the rate limits"));
    assert!(
        requests[0].context_token.is_some(),
        "a summary call must be registered for the module to call back"
    );
    assert!(
        result_text(&result).ends_with("focused note"),
        "the summary replaces the payload: {}",
        result_text(&result)
    );
}

/// A focus captured for one call is not handed to another call's result.
#[tokio::test]
async fn a_focus_belongs_to_its_own_call() {
    let mw = summarizer_mw(StubSummarizer::replying(Ok("note".into())));
    let mut call = TaToolCall::new("a", "web_fetch", json!({"url": "u", "summary_focus": "x"}));
    let mut ctx = ctx();
    mw.before_tool(&mut ctx, &(), &mut call).await.unwrap();

    let mut result = tool_result("web_fetch", &"page ".repeat(200));
    let (_, requests) =
        with_module(mw.after_tool(&mut ctx, &(), &invocation("b", "web_fetch"), &mut result)).await;
    assert_eq!(requests[0].focus, None);
}

/// #6283 review: the authoritative size is stated after the output caps, so a
/// tool cap shorter than the summary cannot cut it away.
#[tokio::test]
async fn the_summarized_size_leads_the_content_for_an_uncapped_tool() {
    // The disclosure of how much was summarized must survive whatever the
    // downstream budget does to the body, so the model is never handed a
    // summary that reads as the whole result.
    //
    // A tool that declares its own cap no longer reaches this stage at all
    // (see `a_tool_that_caps_itself_is_never_sent_to_the_summarizer`), so the
    // interesting case is an uncapped tool whose summary the shared budget
    // then trims.
    let summary = "summary ".repeat(50);
    let mw = summarizer_mw(StubSummarizer::replying(Ok(summary)));
    let raw = "payload ".repeat(200);
    let mut result = tool_result("test_tool", &raw);

    with_module(mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("uncapped-summary", "test_tool"),
        &mut result,
    ))
    .await
    .0
    .unwrap();

    assert!(
        result_text(&result)
            .starts_with("[openhuman: summary of 1600 bytes of tool output, complete]"),
        "the real size must lead the content whatever the caps did, got {:?}",
        result_text(&result).chars().take(160).collect::<String>()
    );
}

/// `summary_focus` belongs to TinyJuice only on a tool that declared it. Any
/// other tool with a parameter of that name (an MCP server's) keeps its value.
#[tokio::test]
async fn a_tool_that_did_not_declare_summary_focus_keeps_the_argument() {
    let mw = summarizer_mw(StubSummarizer::replying(Ok("note".into())));
    let arguments = json!({"query": "q", "summary_focus": "its own meaning"});
    let mut call = TaToolCall::new("mcp-1", "mcp_search", arguments.clone());
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    assert_eq!(call.arguments, arguments);
}

/// With no thread, summary reuse and the breaker are scoped to the run, one
/// scope for every result in it, rather than a fresh one per call.
#[tokio::test]
async fn a_run_without_a_thread_keeps_one_scope() {
    let mut ctx = ctx();
    assert!(ctx.data.thread_id.is_none());
    let mut scopes = Vec::new();
    for id in ["first", "second"] {
        let mw = summarizer_mw(StubSummarizer::replying(Ok("note".into())));
        let mut result = tool_result("test_tool", &"payload ".repeat(200));
        let (_, requests) =
            with_module(mw.after_tool(&mut ctx, &(), &invocation(id, "test_tool"), &mut result))
                .await;
        scopes.push(requests[0].scope.clone());
    }
    assert_eq!(scopes[0], Some(format!("run-{}", ctx.instance_id())));
    assert_eq!(scopes[0], scopes[1]);
}

#[test]
fn repl_tools_are_exempt_from_compaction_but_not_from_the_cap() {
    // Compacting a `juice_find` answer would store it behind a second handle.
    // The byte cap must still apply: each REPL tool declares its own limit.
    for tool in tinyjuice_bus::tools::REPL_TOOLS {
        assert!(is_compaction_exempt(tool), "{tool} must not be compacted");
        assert!(
            !is_truncation_exempt(tool),
            "{tool}'s answer must stay size-capped"
        );
    }
}
