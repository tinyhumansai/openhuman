use super::*;
use tinyagents_harness::context::{RunConfig, RunContext};

fn context() -> RunContext<crate::agent::tinyagents::host::OpenHumanRunContext> {
    RunContext::new(
        RunConfig::new("tool-output-identity-test"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    )
}

#[tokio::test]
async fn same_tool_calls_persist_artifacts_under_distinct_call_ids() {
    let temp = tempfile::tempdir().expect("temporary artifact root");
    let store = crate::agent::harness::tool_result_artifacts::in_memory_tool_result_store(
        temp.path().to_path_buf(),
        "identity-session",
    );
    let middleware = ToolOutputMiddleware {
        budget_bytes: 8,
        payload_summarizer: None,
        artifact_store: Some(store.clone()),
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
    let mut ctx = context();

    let first = ToolInvocationIdentity::new("echo-1", "echo");
    let mut first_result = TaToolResult::success("first result is deliberately oversized");
    middleware
        .after_tool(&mut ctx, &(), &first, &mut first_result)
        .await
        .expect("first artifact is persisted");

    let second = ToolInvocationIdentity::new("echo-2", "echo");
    let mut second_result = TaToolResult::success("second result is deliberately oversized");
    middleware
        .after_tool(&mut ctx, &(), &second, &mut second_result)
        .await
        .expect("second artifact is persisted");

    assert_eq!(
        store
            .read("tool-results/identity-session/echo/echo-1.txt")
            .await
            .unwrap()
            .expect("first artifact"),
        "first result is deliberately oversized"
    );
    assert_eq!(
        store
            .read("tool-results/identity-session/echo/echo-2.txt")
            .await
            .unwrap()
            .expect("second artifact"),
        "second result is deliberately oversized"
    );
}

/// `raw: true` asks `web_fetch` for the body as sent, which switches off the
/// HTML→Markdown conversion — so the payload is unconverted markup, and handing
/// it to the summarizer buys an uncached model call to paraphrase minified JS.
/// One observed fetch cost 44,561 prompt tokens that way. These pin which calls
/// earn the exemption, not what the ladder then does with them.
#[test]
fn only_a_raw_web_fetch_is_exempt_from_the_payload_summarizer() {
    use serde_json::json;

    assert!(is_raw_fetch(
        "web_fetch",
        &json!({"url": "https://x", "raw": true})
    ));

    // A converted fetch is the normal path and stays summarizer-eligible: its
    // Markdown is prose the summarizer compresses well.
    for args in [
        json!({"url": "https://x"}),
        json!({"url": "https://x", "raw": false}),
        json!({"url": "https://x", "raw": null}),
        // `raw` is a bool on the wire; a string is not a request for raw bytes.
        json!({"url": "https://x", "raw": "true"}),
    ] {
        assert!(
            !is_raw_fetch("web_fetch", &args),
            "{args} is a converted fetch"
        );
    }

    // The exemption is about `web_fetch`'s conversion, so a `raw` argument on
    // any other tool means nothing here.
    for tool in ["file_read", "shell", "http_request"] {
        assert!(
            !is_raw_fetch(tool, &json!({"raw": true})),
            "{tool} has no HTML conversion to switch off"
        );
    }
}

/// `use_skill` forwards the wrapped tool's result verbatim, so a raw fetch
/// reached through it is still a raw fetch — the same wrapper-following
/// `artifact_read_target` does.
#[test]
fn a_raw_fetch_wrapped_in_use_skill_is_still_a_raw_fetch() {
    use serde_json::json;

    assert!(is_raw_fetch(
        "use_skill",
        &json!({"skill": "web", "tool": "web_fetch", "args": {"url": "https://x", "raw": true}})
    ));
    assert!(!is_raw_fetch(
        "use_skill",
        &json!({"skill": "web", "tool": "web_fetch", "args": {"url": "https://x"}})
    ));
    // A wrapper naming some other tool, and a malformed one, are not raw
    // fetches — neither may silently inherit the exemption.
    assert!(!is_raw_fetch(
        "use_skill",
        &json!({"skill": "files", "tool": "file_read", "args": {"raw": true}})
    ));
    assert!(!is_raw_fetch("use_skill", &json!({"raw": true})));
}

fn policy_mw(budget: usize, caps: &[(&str, usize)]) -> ToolOutputMiddleware {
    let mut tool_policies = HashMap::new();
    for (name, cap) in caps {
        let mut policy = TaToolPolicy::default();
        policy.runtime.max_result_bytes = Some(*cap);
        tool_policies.insert((*name).to_string(), policy);
    }
    ToolOutputMiddleware {
        budget_bytes: budget,
        payload_summarizer: None,
        artifact_store: None,
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Full,
        runtime_config: None,
        tool_policies,
        artifact_reads: Default::default(),
        focus_by_call: Default::default(),
        summary_focus_tools: Default::default(),
        raw_fetches: Default::default(),
        file_reads: Default::default(),
    }
}

#[test]
fn a_tools_declared_cap_wins_over_the_shared_budget_in_the_stage_plan() {
    let mw = policy_mw(16_000, &[("web_fetch", 50_000)]);
    let declared = plan_output_stages(&mw, "web_fetch", 10, false);
    assert_eq!(declared.budget_bytes, 50_000);
    assert!(declared.tool_declared_cap);
    let shared = plan_output_stages(&mw, "shell", 10, false);
    assert_eq!(shared.budget_bytes, 16_000);
    assert!(!shared.tool_declared_cap);
}

#[test]
fn proposal_and_discovery_tools_are_exempt_from_the_byte_cap() {
    let mw = policy_mw(100, &[]);
    for tool in ["propose_workflow", "mcp_list_tools"] {
        assert!(plan_output_stages(&mw, tool, 10_000, true).truncation_exempt);
    }
    // A sampling tool skips compaction but stays under the byte cap.
    let sampling = plan_output_stages(&mw, "get_tool_contract", 10_000, true);
    assert!(sampling.compaction_exempt && !sampling.truncation_exempt);
}

#[test]
fn the_untouched_output_is_kept_only_when_it_could_spill_and_be_read_back() {
    let mw = policy_mw(100, &[]);
    assert!(plan_output_stages(&mw, "shell", 101, true).keep_full_output);
    assert!(!plan_output_stages(&mw, "shell", 101, false).keep_full_output);
    assert!(!plan_output_stages(&mw, "shell", 100, true).keep_full_output);
    let too_big = tinytools_std::filesystem::FileReadTool::MAX_FILE_SIZE_BYTES as usize + 1;
    assert!(!plan_output_stages(&mw, "shell", too_big, true).keep_full_output);
}
