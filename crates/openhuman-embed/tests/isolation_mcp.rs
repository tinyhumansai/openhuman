//! MCP servers belong to the agent that installed them: two agents on one
//! workspace keep separate installed lists and connections, and an embedded
//! agent whose id is not the orchestrator's still searches its own connected
//! MCP tools.

#![cfg(feature = "mcp")]

mod common;

use std::sync::Arc;

use common::{chat_requests, offline_config, route, runtime, scripted_provider, stub_backend};
use openhuman_core::core::runtime::{AgentContextRegistry, CoreContext};
use openhuman_core::mcp::registry::types::{CommandKind, InstalledServer, Transport};
use openhuman_embed::{Access, AgentSpec, Runtime, Workspace};
use serde_json::{json, Value};
use tinytools::Tool;

const MCP_TOOL: &str = "isolation_forecast";

struct LoopbackMcp;

impl wiremock::Respond for LoopbackMcp {
    fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let result = match body["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "loopback", "version": "1.0.0" },
            }),
            "notifications/initialized" => return wiremock::ResponseTemplate::new(202),
            "tools/list" => json!({
                "tools": [{
                    "name": MCP_TOOL,
                    "description": "Forecast lookups for the isolation test.",
                    "inputSchema": { "type": "object" }
                }]
            }),
            "tools/call" => json!({ "content": [{ "type": "text", "text": "sunny" }] }),
            _ => json!({}),
        };
        wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": body["id"].clone(),
            "result": result,
        }))
    }
}

fn loopback_server(url: String) -> InstalledServer {
    InstalledServer {
        server_id: "loopback-1".into(),
        qualified_name: "loopback".into(),
        display_name: "loopback".into(),
        description: None,
        icon_url: None,
        command_kind: CommandKind::Node,
        command: String::new(),
        args: Vec::new(),
        env_keys: Vec::new(),
        config: None,
        installed_at: 1,
        last_connected_at: None,
        transport: Transport::HttpRemote { url },
        enabled: true,
    }
}

fn context_of(agent_id: &str) -> Arc<CoreContext> {
    AgentContextRegistry::get(agent_id).expect("the agent's context is registered")
}

async fn installed_ids(agent_id: &str, config: &openhuman_core::config::Config) -> Vec<String> {
    let config = config.clone();
    CoreContext::scope(context_of(agent_id), async move {
        openhuman_core::mcp::host::for_config(&config)
            .expect("the agent's host opens")
            .dynamic()
            .store()
            .list_servers()
            .expect("installed servers list")
            .into_iter()
            .map(|server| server.server_id)
            .collect()
    })
    .await
}

async fn connected_names(agent_id: &str) -> Vec<String> {
    CoreContext::scope(context_of(agent_id), async {
        openhuman_core::mcp::registry::connections::connected_overview()
            .await
            .into_iter()
            .map(|server| server.qualified_name)
            .collect()
    })
    .await
}

fn tool_search_call() -> Value {
    common::tool_call_completion(
        "tool_search",
        &json!({ "query": "isolation forecast" }).to_string(),
    )
}

#[test]
fn mcp_servers_stay_with_the_agent_that_installed_them() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let mcp = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(LoopbackMcp)
                .mount(&mcp)
                .await;
            let runtime_provider = scripted_provider(Vec::new(), "runtime-provider").await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .provider(route(&runtime_provider, "runtime-model"))
                .access(Access::full())
                .build()
                .await
                .expect("runtime builds");

            let a_provider = scripted_provider(vec![tool_search_call()], "a-done").await;
            let b_provider = scripted_provider(vec![tool_search_call()], "b-done").await;
            let a = runtime
                .agent(
                    AgentSpec::new("mcp-a")
                        .provider(route(&a_provider, "a-model"))
                        .access(Access::full()),
                )
                .expect("a instantiates");
            let b = runtime
                .agent(
                    AgentSpec::new("mcp-b")
                        .provider(route(&b_provider, "b-model"))
                        .access(Access::full()),
                )
                .expect("b instantiates");
            assert_eq!(a.workspace_dir(), b.workspace_dir());

            // ── A installs and connects a server in its own host ──
            let a_config = a.config().clone();
            let url = format!("{}/mcp", mcp.uri());
            CoreContext::scope(context_of("mcp-a"), async move {
                let host = openhuman_core::mcp::host::for_config(&a_config).expect("a's host");
                host.dynamic()
                    .store()
                    .insert_server(&loopback_server(url))
                    .expect("install into a's store");
                openhuman_core::mcp::registry::tools::McpRegistryConnectTool::new(Arc::new(
                    a_config,
                ))
                .execute(json!({ "server_id": "loopback-1" }))
                .await
                .expect("a connects its server");
            })
            .await;

            assert_eq!(installed_ids("mcp-a", a.config()).await, ["loopback-1"]);
            assert!(
                installed_ids("mcp-b", b.config()).await.is_empty(),
                "b's installed list stays empty"
            );
            assert_eq!(connected_names("mcp-a").await, ["loopback"]);
            assert!(connected_names("mcp-b").await.is_empty());
            let workspace_host = openhuman_core::mcp::host::for_config(a.config())
                .expect("the workspace host opens");
            assert!(
                workspace_host
                    .dynamic()
                    .store()
                    .list_servers()
                    .expect("workspace list")
                    .is_empty(),
                "the runtime's own host never sees a's server"
            );

            // ── A's turn finds its MCP tool; B's does not ──
            a.run("find the forecast tool").await.expect("a's turn");
            b.run("find the forecast tool").await.expect("b's turn");

            let a_results = chat_requests(&a_provider)
                .await
                .iter()
                .map(common::tool_results)
                .collect::<String>();
            assert!(
                a_results.contains(MCP_TOOL),
                "a's tool search reaches its connected MCP tool: {a_results}"
            );
            let b_results = chat_requests(&b_provider)
                .await
                .iter()
                .map(common::tool_results)
                .collect::<String>();
            assert!(
                !b_results.contains(MCP_TOOL),
                "b never sees a's MCP tool: {b_results}"
            );
            assert_eq!(chat_requests(&runtime_provider).await.len(), 0);
        })
        .await
        .expect("test task");
    });
}
