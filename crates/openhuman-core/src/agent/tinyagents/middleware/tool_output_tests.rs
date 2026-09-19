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
    let middleware = ToolOutputMiddleware {
        budget_bytes: 8,
        payload_summarizer: None,
        task_hint: None,
        artifact_store: Some(ToolResultArtifactStore::new(
            temp.path().to_path_buf(),
            "identity-session",
        )),
        tokenjuice_compaction_enabled: false,
        tokenjuice_compression: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        tool_policies: HashMap::new(),
        artifact_reads: Default::default(),
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

    let root = temp
        .path()
        .join("artifacts/tool-results/identity-session/echo");
    assert_eq!(
        std::fs::read_to_string(root.join("echo-1.txt")).expect("first artifact"),
        "first result is deliberately oversized"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("echo-2.txt")).expect("second artifact"),
        "second result is deliberately oversized"
    );
}
