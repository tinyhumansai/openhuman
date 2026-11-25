//! Host smoke test for the tinymcp module: an embedded runtime that declares an
//! HTTP MCP server reaches a tool on it through the module.
//!
//! The host supplies everything a released module gets: a staged verified
//! module artifact, a SQLite storage backend, and a grant covering what the
//! module's client asks for when it connects. Without any one of them the
//! module faults at startup and the call answers "TinyMCP module is
//! unavailable" (tinyhumansai/openhuman#8280).

mod common;

use common::{offline_config, route, scripted_provider, stub_backend};
use openhuman_core::config::schema::ModuleOverride;
use openhuman_embed::{
    Access, AgentSpec, McpServer, Runtime, RuntimeModule, ServiceSet, Workspace,
};

struct McpStub;

impl wiremock::Respond for McpStub {
    fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let result = match body["method"].as_str().unwrap_or_default() {
            "initialize" => serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "forecast", "version": "1" }
            }),
            "notifications/initialized" => return wiremock::ResponseTemplate::new(202),
            "tools/list" => serde_json::json!({ "tools": [{
                "name": "forecast",
                "description": "Local forecast",
                "inputSchema": { "type": "object" }
            }] }),
            "tools/call" => serde_json::json!({ "content": [{ "type": "text", "text": "sunny" }] }),
            _ => serde_json::json!({}),
        };
        wiremock::ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "jsonrpc": "2.0", "id": body["id"], "result": result }),
        )
    }
}

#[test]
fn an_embedded_runtime_calls_a_tool_through_the_tinymcp_module() {
    let _ = env_logger::builder().is_test(true).try_init();
    // The turn's future is large; `block_on` polls it on this thread, which
    // would otherwise have the default test stack.
    std::thread::Builder::new()
        .stack_size(openhuman_embed::process::AGENT_WORKER_STACK_BYTES)
        .spawn(scenario)
        .expect("spawn")
        .join()
        .expect("scenario");
}

fn scenario() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(openhuman_embed::process::AGENT_WORKER_STACK_BYTES)
        .build()
        .expect("tokio runtime");
    rt.block_on(async {
        let backend = stub_backend().await;
        let mcp = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(McpStub)
            .mount(&mcp)
            .await;
        let model = scripted_provider(
            vec![common::tool_call_completion(
                "mcp_call_tool",
                r#"{"server":"forecast","tool":"forecast","arguments":{}}"#,
            )],
            "done",
        )
        .await;
        let mut config = offline_config();
        config.modules.allow_download = false;
        let module_path =
            std::env::var("OPENHUMAN_TINYMCP_TEST_MODULE").expect("staged TinyMCP module artifact");
        config
            .modules
            .overrides
            .push(ModuleOverride::with_path("tinymcp", module_path));
        let storage_dir = tempfile::tempdir().expect("SQLite storage directory");
        let storage_url = openhuman_core::storage::config::default_url(storage_dir.path());
        let runtime = Runtime::builder()
            .config(config)
            .workspace(Workspace::Ephemeral)
            .storage(storage_url)
            .backend_url(backend.uri())
            .provider(route(&model, "fixture"))
            .services(ServiceSet::none())
            .modules([
                RuntimeModule::Agent,
                RuntimeModule::Inference,
                RuntimeModule::Modules,
                RuntimeModule::Memory,
                RuntimeModule::Mcp,
                RuntimeModule::Skills,
            ])
            .build()
            .await
            .expect("runtime builds");
        let agent = runtime
            .agent(
                AgentSpec::new("mcp-client")
                    .provider(route(&model, "fixture"))
                    .mcp(
                        McpServer::http("forecast", format!("{}/mcp", mcp.uri()))
                            .allow_tools(["forecast"]),
                    )
                    .access(Access::full()),
            )
            .expect("agent");
        let reply = agent.run("call forecast").await.unwrap_or_else(|error| {
            eprintln!("TinyMCP embed turn failed: {error:#}");
            panic!("turn failed")
        });
        let requests = common::chat_requests(&model).await;
        let results = common::tool_results(requests.last().expect("a follow-up request"));
        let methods: Vec<String> = mcp
            .received_requests()
            .await
            .expect("recorded")
            .iter()
            .filter_map(|r| {
                serde_json::from_slice::<serde_json::Value>(&r.body).ok()?["method"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect();
        if !methods.iter().any(|method| method == "tools/call") {
            eprintln!("TinyMCP methods={methods:?}; tool results={results}");
        }
        assert_eq!(reply.reply, "done");
        assert!(
            methods.iter().any(|m| m == "tools/call"),
            "{methods:?} / {results}"
        );
        assert!(results.contains("sunny"), "{results}");
    });
}
