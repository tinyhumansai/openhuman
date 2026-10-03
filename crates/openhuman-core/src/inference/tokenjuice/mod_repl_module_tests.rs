//! Handle mode and the REPL tools over the real bus. Runs where CI carries the
//! released module (`TINYJUICE_TEST_MODULE`); skipped otherwise.

use super::*;

/// A large result comes back as a stats line, a head and a handle, and
/// `juice_find` reads the stored original through `Retrieve`.
#[tokio::test]
async fn a_large_result_becomes_a_handle_the_repl_tools_can_query() {
    if std::env::var_os("TINYJUICE_TEST_MODULE").is_none() {
        eprintln!(
            "SKIPPED (not run, not asserted): TINYJUICE_TEST_MODULE is not set. Build \
             vendor/tinyjuice and export TINYJUICE_TEST_MODULE=<path to libtinyjuice_module>, \
             or use scripts/test-rust-with-mock.sh"
        );
        return;
    }
    let rows: String = (0..1200)
        .map(|i| format!("row {i}: value {}\n", i * 7))
        .collect();
    let content = format!("# Report\n{rows}ERROR: needle in the middle\n{rows}");

    let output = compact_tool_output(ToolOutputCompaction {
        content: content.clone(),
        tool_name: "shell",
        enabled: true,
        profile: AgentTokenjuiceCompression::Full,
        runtime_config: None,
        arguments: None,
        focus: None,
        context_token: None,
        scope: Some("module-repl-test".into()),
    })
    .await;

    assert!(
        output.text.len() < content.len() / 4,
        "not compacted: {} bytes",
        output.text.len()
    );
    // The footer advertises slice/search, outline, and full recovery. Structured
    // extraction remains callable, but is not a suggested recovery operation.
    for name in ["juice_find", "juice_summarize", "juice_retrieve"] {
        assert!(output.text.contains(name), "footer must name {name}");
    }
    let handle = output
        .text
        .split("handle \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the footer names a handle")
        .to_string();

    let tools = repl_tools();
    let find = tools.iter().find(|t| t.name() == "juice_find").unwrap();
    let hit = find
        .execute(serde_json::json!({ "handle": handle, "query": "needle" }))
        .await
        .unwrap();
    assert!(!hit.is_error, "{}", hit.output());
    assert!(hit.output().contains("needle in the middle"));

    let gone = find
        .execute(serde_json::json!({ "handle": "0123456789abcdef0123456789abcdef", "query": "x" }))
        .await
        .unwrap();
    assert!(gone.is_error);
}
