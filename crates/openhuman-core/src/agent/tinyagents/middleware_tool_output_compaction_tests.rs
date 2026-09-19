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
#[ignore = "requires a built TinyJuice module"]
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
        payload.len() > DEFAULT_TOOL_RESULT_BUDGET_BYTES,
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

/// #6283: the summarizer used to be called with a hard-coded `None` hint, so
/// it compressed every payload blind to what the user asked for.
#[tokio::test]
async fn the_turns_task_hint_reaches_the_payload_summarizer() {
    struct HintRecorder(std::sync::Mutex<Option<Option<String>>>);
    #[async_trait]
    impl PayloadSummarizer for HintRecorder {
        async fn maybe_summarize_in_parent(
            &self,
            _parent_ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
            _tool_name: &str,
            parent_task_hint: Option<&str>,
            _raw: &str,
        ) -> anyhow::Result<SummarizeOutcome> {
            *self.0.lock().expect("recorder lock") = Some(parent_task_hint.map(str::to_owned));
            Ok(SummarizeOutcome::NotNeeded)
        }
    }

    let recorder = Arc::new(HintRecorder(std::sync::Mutex::new(None)));
    let mut mw = summarizer_mw(recorder.clone());
    mw.task_hint = Some("find the release notes for v2".to_string());
    let mut result = tool_result("use_skill", "RAW-TOOL-OUTPUT");

    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("use-skill", "use_skill"),
        &mut result,
    )
    .await
    .expect("after_tool should not fail");

    assert_eq!(
        recorder.0.lock().expect("recorder lock").clone(),
        Some(Some("find the release notes for v2".to_string())),
        "the turn's task hint must be handed to the payload summarizer"
    );
}

/// #6283 review: the authoritative size is stated after the output caps, so a
/// tool cap shorter than the summary cannot cut it away.
#[tokio::test]
async fn the_summarized_size_survives_a_tool_cap_shorter_than_the_summary() {
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
            max_result_bytes: Some(12),
            streaming: false,
        }),
    );
    let summary = "summary ".repeat(50);
    let mut mw = summarizer_mw(StubSummarizer::ok(SummarizeOutcome::Summarized(
        crate::agent::tinyagents::payload_summarizer::SummarizedPayload {
            summary_bytes: summary.len(),
            summary,
            original_bytes: 119_796,
        },
    )));
    mw.tool_policies = tool_policies;
    let mut result = tool_result("terse", &"payload ".repeat(200));

    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("terse-cap", "terse"),
        &mut result,
    )
    .await
    .unwrap();

    assert!(
        result_text(&result)
            .starts_with("[openhuman: summary of 119796 bytes of tool output, complete]"),
        "the real size must lead the content whatever the caps did, got {:?}",
        result_text(&result).chars().take(160).collect::<String>()
    );
    assert!(
        result_text(&result).contains("[truncated by tool cap:"),
        "the summary itself is still bound by the tool's cap"
    );
}
