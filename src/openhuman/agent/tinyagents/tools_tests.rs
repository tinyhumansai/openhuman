use super::*;
use crate::openhuman::tools::traits::ToolTimeout;
use crate::openhuman::tools::ToolResult as OhToolResult;

/// A tool whose `execute_with_options` sleeps forever but declares a short
/// per-call timeout, so the adapter's deadline must fire.
struct HangingTool;

#[async_trait]
impl crate::openhuman::tools::Tool for HangingTool {
    fn name(&self) -> &str {
        "hang"
    }
    fn description(&self) -> &str {
        "hangs"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<OhToolResult> {
        futures_util::future::pending::<()>().await;
        Ok(OhToolResult::success("never"))
    }
    fn timeout_policy(&self, _args: &serde_json::Value) -> ToolTimeout {
        ToolTimeout::Secs(1)
    }
}

/// A fast tool that echoes an argument, to prove the normal path still runs.
struct EchoTool;

#[async_trait]
impl crate::openhuman::tools::Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<OhToolResult> {
        let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
        Ok(OhToolResult::success(format!("echoed:{m}")))
    }
}

fn call(name: &str, args: serde_json::Value) -> TaToolCall {
    TaToolCall {
        id: "c1".into(),
        name: name.into(),
        arguments: args,
        invalid: None,
    }
}

#[tokio::test]
async fn tool_execution_respects_the_per_call_timeout() {
    let result =
        execute_openhuman_tool(&HangingTool, call("hang", serde_json::json!({})), None).await;
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("timed out")),
        "a hanging tool must surface a timeout error, got {:?}",
        result.error
    );
    assert!(result.content.contains("timed out"));
}

#[tokio::test]
async fn fast_tool_runs_to_completion() {
    let result = execute_openhuman_tool(
        &EchoTool,
        call("echo", serde_json::json!({ "msg": "hi" })),
        None,
    )
    .await;
    assert!(result.error.is_none());
    assert!(result.content.contains("echoed:hi"));
}
