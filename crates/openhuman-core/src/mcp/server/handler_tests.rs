use serde_json::{json, Map, Value};
use tinymcp::{McpServerHandler, RequestContext, RequestHeaders, ToolCallError};

use super::*;
use crate::mcp::server::subagent_depth::{HEADER_SUBAGENT_DEPTH, MAX_SUBAGENT_DEPTH};

fn context(headers: RequestHeaders) -> RequestContext {
    RequestContext::new("mcp:test", headers)
}

#[test]
fn server_info_is_openhumans_identity_and_guidance() {
    let info = OpenHumanMcpHandler.server_info();
    assert_eq!(info.name, "openhuman-core");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    let instructions = info.instructions.expect("OpenHuman gives instructions");
    assert!(instructions.contains("core.list_tools"), "{instructions}");
}

#[test]
fn sessions_are_attributed_under_the_mcp_prefix() {
    assert_eq!(OpenHumanMcpHandler.source_type_prefix(), "mcp");
}

#[tokio::test]
async fn list_tools_advertises_the_base_catalog_with_annotations() {
    let tools = OpenHumanMcpHandler
        .list_tools(&context(RequestHeaders::new()))
        .await;
    let names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "core.list_tools",
        "agent.run_subagent",
        "memory.recall",
        "memory.forget",
    ] {
        assert!(names.contains(&expected), "missing {expected}: {names:?}");
    }
    assert!(tools
        .iter()
        .all(|tool| tool.title.is_some() && tool.annotations.is_some()));
}

#[tokio::test]
async fn resources_are_the_bundled_prompt_catalog() {
    let resources = OpenHumanMcpHandler.list_resources();
    assert!(resources
        .iter()
        .all(|resource| resource.mime_type.as_deref() == Some("text/markdown")));
    let identity = OpenHumanMcpHandler
        .read_resource("openhuman://prompts/identity")
        .await
        .expect("identity resource");
    assert_eq!(
        identity["contents"][0]["uri"],
        "openhuman://prompts/identity"
    );
    let err = OpenHumanMcpHandler
        .read_resource("openhuman://prompts/nope")
        .await
        .expect_err("unknown uri");
    assert_eq!(
        err,
        ToolCallError::ResourceNotFound("no resource with uri `openhuman://prompts/nope`".into())
    );
}

#[tokio::test]
async fn call_tool_runs_inside_the_delegation_depth_the_request_carries() {
    // At the cap, `agent.run_subagent` refuses before building anything —
    // which only happens if the header reached the depth scope.
    let mut headers = RequestHeaders::new();
    headers.insert(HEADER_SUBAGENT_DEPTH, MAX_SUBAGENT_DEPTH.to_string());
    let arguments = match json!({"agent_id": "planner", "prompt": "hi"}) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let err = OpenHumanMcpHandler
        .call_tool(&context(headers), "agent.run_subagent", arguments)
        .await
        .expect_err("depth cap refuses");
    assert!(
        err.message().contains("delegation depth limit reached"),
        "{err:?}"
    );
}

#[tokio::test]
async fn call_tool_rejects_unknown_tools_as_invalid_params() {
    let err = OpenHumanMcpHandler
        .call_tool(&context(RequestHeaders::new()), "no.such_tool", Map::new())
        .await
        .expect_err("unknown tool");
    assert_eq!(
        err,
        ToolCallError::InvalidParams("unknown MCP tool `no.such_tool`".into())
    );
}

#[cfg(feature = "http-server")]
#[tokio::test]
async fn the_http_client_round_trips_against_the_openhuman_handler() {
    let endpoint = super::super::test_support::spawn_http(None).await;
    let client = crate::mcp::http_client::McpHttpClient::new(endpoint, 5).expect("client");
    let init = client.initialize().await.expect("initialize");
    assert_eq!(init.server_info["name"], "openhuman-core");
    let tools = client.list_tools().await.expect("tools/list");
    assert!(tools.iter().any(|tool| tool.name == "memory.recall"));
    client.close_session().await.expect("DELETE session");
}
