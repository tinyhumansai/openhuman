//! Integration tests for the MCP setup agent.
//!
//! These tests hit real MCP servers via stdio (npx/uvx) and verify the
//! full handshake + tool discovery works end-to-end. They require network
//! access and the `npx`/`uvx` binaries to be available.
//!
//! Run with: `cargo test --lib -- setup_agent_integration --ignored`

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::openhuman::config::McpClientIdentityConfig;
    use crate::openhuman::mcp_client::setup_agent::{resolve_setup_request, test_connection};

    fn test_identity() -> McpClientIdentityConfig {
        McpClientIdentityConfig {
            name: "openhuman-test".to_string(),
            title: "OpenHuman Test".to_string(),
            version: "0.0.1".to_string(),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn npm_memory_server_connects_and_lists_tools() {
        let req = resolve_setup_request(
            "npm",
            "@modelcontextprotocol/server-memory",
            Some("npx"),
            &["-y".to_string()],
            HashMap::new(),
        );
        assert_eq!(req.command, "npx");

        let result = test_connection(&req, test_identity())
            .await
            .expect("npm memory server should connect");

        assert_eq!(result.server_name, "memory-server");
        assert!(
            !result.tools.is_empty(),
            "memory server should expose tools"
        );
        assert!(
            result
                .tools
                .iter()
                .any(|t| t.name == "read_graph" || t.name == "create_entities"),
            "expected knowledge-graph tools, got: {:?}",
            result.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn pypi_time_server_connects_and_lists_tools() {
        let req = resolve_setup_request("pypi", "mcp-server-time", None, &[], HashMap::new());
        assert_eq!(req.command, "uvx");
        assert_eq!(req.args, vec!["mcp-server-time"]);

        let result = test_connection(&req, test_identity())
            .await
            .expect("pypi time server should connect");

        assert_eq!(result.server_name, "mcp-time");
        assert!(!result.tools.is_empty(), "time server should expose tools");
    }
}
