use super::*;

#[test]
fn async_completion_detects_unexecuted_tool_call_stubs() {
    assert!(is_unexecuted_tool_call_stub(
        "<tool_call>{\"name\": \"GMAIL_LIST_MESSAGES\", \"arguments\": {}}</tool_call>"
    ));
    assert!(is_unexecuted_tool_call_stub(
        "{\n  \"tool_calls\": [{\"name\": \"GMAIL_FETCH_EMAILS\"}]\n}"
    ));
    assert!(!is_unexecuted_tool_call_stub(
        "I found 3 job emails. I used <tool_call>GMAIL_FETCH_EMAILS</tool_call> to read them."
    ));
    assert!(!is_unexecuted_tool_call_stub(
        "The \"tool_use\" field is how the provider reports a call."
    ));
}

#[test]
fn delegated_failures_keep_the_terminal_inference_failure_marker() {
    let error = format_subagent_failure(
        "spawn_subagent",
        "all providers/models failed: model 'example' returned HTTP 404",
    );
    assert!(error.contains("spawn_subagent failed and did not complete"));
    assert!(error.contains("Do not treat this as success"));
    assert!(error.contains("fabricate output"));
    assert!(error.contains("all providers/models failed"));
}
