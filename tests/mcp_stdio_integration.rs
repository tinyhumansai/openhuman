//! Integration coverage for the stdio MCP client against the real core binary.
//!
//! Keep this as an integration test so Cargo builds `openhuman-core` as part of
//! the test graph and exposes it through `CARGO_BIN_EXE_openhuman-core`. Running
//! a nested `cargo build` from a lib unit test is prone to CI disk exhaustion.

use openhuman_core::openhuman::config::McpClientIdentityConfig;
use openhuman_core::openhuman::mcp_client::McpStdioClient;
use std::path::PathBuf;

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

#[tokio::test]
async fn stdio_client_talks_to_openhuman_mcp_server() {
    let client = McpStdioClient::new(
        env!("CARGO_BIN_EXE_openhuman-core").to_string(),
        vec!["mcp".into()],
        Vec::new(),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
        McpClientIdentityConfig::default(),
    );

    let init = client.initialize().await.expect("initialize");
    assert_eq!(init.protocol_version, LATEST_PROTOCOL_VERSION);

    let tools = client.list_tools().await.expect("list_tools");
    assert!(tools.iter().any(|tool| tool.name == "memory.search"));

    client.close_session().await.expect("close");
}
