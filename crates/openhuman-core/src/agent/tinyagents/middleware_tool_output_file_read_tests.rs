//! `ToolOutputMiddleware` and file reads: a call that only prints files the
//! agent can re-read never gets an LLM payload summary. Within the result
//! budget it is left verbatim; past it TinyJuice compacts it deterministically.

use super::*;
use crate::agent::tinyagents::middleware::tool_output::ToolOutputMiddleware;

/// Run one shell call with `command` through `before_tool` and `after_tool`.
async fn run_shell(
    mw: &ToolOutputMiddleware,
    id: &str,
    command: &str,
    output: &str,
) -> (String, Vec<tinyjuice_bus::wire::CompactRequest>) {
    let mut ctx = ctx();
    let mut call = TaToolCall::new(id, "shell", json!({ "command": command }));
    mw.before_tool(&mut ctx, &(), &mut call).await.unwrap();
    let mut result = tool_result("shell", output);
    let (outcome, requests) =
        with_module(mw.after_tool(&mut ctx, &(), &invocation(id, "shell"), &mut result)).await;
    outcome.expect("after_tool succeeds");
    (result_text(&result), requests)
}

#[tokio::test]
async fn a_file_read_within_the_budget_is_left_verbatim() {
    let stub = StubSummarizer::replying(Ok("a paraphrase of the file".into()));
    let mut mw = summarizer_mw(stub.clone());
    mw.tokenjuice_compaction_enabled = true;
    mw.budget_bytes = 16 * 1024;
    let file = "package vm\n\nfunc funcExpr() {}\n".repeat(400); // ~13 KB
    let (text, requests) =
        run_shell(&mw, "read-1", "cd /app && cat vm/vmExprFunction.go", &file).await;

    assert_eq!(text, file, "the agent asked for the file's exact text");
    assert!(!stub.was_prepared(), "no summary call for a file read");
    assert!(
        requests.is_empty(),
        "nothing to ask TinyJuice within the budget"
    );
}

#[tokio::test]
async fn a_zero_budget_leaves_file_reads_verbatim() {
    let stub = StubSummarizer::replying(Ok("a paraphrase of the file".into()));
    let mut mw = summarizer_mw(stub.clone());
    mw.tokenjuice_compaction_enabled = true;
    mw.budget_bytes = 0;
    let file = "package vm\\n".repeat(400);
    let (text, requests) =
        run_shell(&mw, "read-zero-budget", "cat vm/vmExprFunction.go", &file).await;

    assert_eq!(text, file);
    assert!(!stub.was_prepared());
    assert!(requests.is_empty());
}

#[tokio::test]
async fn a_file_read_past_the_budget_is_compacted_without_a_summary() {
    let stub = StubSummarizer::replying(Ok("a paraphrase of the file".into()));
    let mut mw = summarizer_mw(stub.clone());
    mw.tokenjuice_compaction_enabled = true;
    mw.budget_bytes = 4 * 1024;
    let file = "package vm\n\nfunc funcExpr() {}\n".repeat(400);
    let (_, requests) =
        run_shell(&mw, "read-2", "sed -n '1,400p' vm/vmExprFunction.go", &file).await;

    assert!(!stub.was_prepared(), "no summary call for a file read");
    assert_eq!(requests.len(), 1, "TinyJuice still compacts it");
    assert!(
        requests[0].context_token.is_none(),
        "and is given no summary call to make"
    );
}

#[tokio::test]
async fn other_shell_output_is_still_summarized() {
    let stub = StubSummarizer::replying(Ok("tests: 3 failed".into()));
    let mw = summarizer_mw(stub.clone());
    let log = "--- FAIL: TestDefaults (0.00s)\n".repeat(400);
    let (text, _) = run_shell(&mw, "test-1", "go test ./vm/...", &log).await;

    assert!(stub.was_prepared(), "a test log is not a file on disk");
    assert!(text.contains("tests: 3 failed"), "{text}");
}
