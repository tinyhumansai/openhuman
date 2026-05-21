//! MCP stdio client: spawns a child process and speaks the MCP JSON-RPC
//! stdio protocol (initialize → tools/list → tools/call).
//!
//! The client is `Send + Sync` and is stored in a global registry keyed by
//! `server_id`. Callers use the `McpTransport` trait for testing (see
//! `protocol::McpTransport`).

pub mod protocol;
pub mod transport;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::openhuman::mcp_clients::types::McpTool;

use protocol::{
    build_initialize_params, build_request, parse_tools_list, send_request_and_wait, McpTransport,
    RequestIdCounter,
};
use transport::SpawnedProcess;

// ── McpStdioClient ─────────────────────────────────────────────────────────

/// A live connection to an MCP server over stdio.
pub struct McpStdioClient {
    server_id: String,
    process: Mutex<SpawnedProcess>,
    counter: RequestIdCounter,
    /// Cached tool list after `initialize`.
    cached_tools: Mutex<Vec<McpTool>>,
}

impl McpStdioClient {
    /// Spawn the server process and run `initialize` + `tools/list`.
    pub async fn spawn_and_init(
        server_id: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> anyhow::Result<Arc<Self>> {
        tracing::debug!(
            "[mcp-client] spawn_and_init server_id={} command={} args={:?} env_keys={:?}",
            server_id,
            command,
            args,
            env.keys().collect::<Vec<_>>()
        );

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("[mcp-client] failed to spawn {command}: {e}"))?;

        let proc = SpawnedProcess::from_child(child, server_id)?;
        let client = Arc::new(Self {
            server_id: server_id.to_string(),
            process: Mutex::new(proc),
            counter: RequestIdCounter::new(),
            cached_tools: Mutex::new(Vec::new()),
        });

        // Run MCP initialize handshake
        let init_result = client.initialize().await.map_err(|e| {
            anyhow::anyhow!("[mcp-client] server_id={server_id} initialize failed: {e}")
        })?;
        tracing::debug!(
            "[mcp-client] server_id={} initialize result: {}",
            server_id,
            init_result
        );

        // Discover tools
        let tools = client.list_tools().await.map_err(|e| {
            anyhow::anyhow!("[mcp-client] server_id={server_id} tools/list failed: {e}")
        })?;
        tracing::debug!(
            "[mcp-client] server_id={} discovered {} tools",
            server_id,
            tools.len()
        );
        {
            let mut guard = client.cached_tools.lock().await;
            *guard = tools;
        }

        Ok(client)
    }

    /// Return a snapshot of the cached tool list without a live RPC call.
    pub async fn tools_snapshot(&self) -> Vec<McpTool> {
        self.cached_tools.lock().await.clone()
    }

    /// Return the last stderr line for error reporting.
    pub async fn last_error(&self) -> Option<String> {
        self.process.lock().await.reader.last_stderr().await
    }
}

#[async_trait]
impl McpTransport for McpStdioClient {
    async fn initialize(&self) -> Result<Value, String> {
        let id = self.counter.next().await;
        let msg = build_request(id, "initialize", Some(build_initialize_params()));

        let result = {
            let mut proc = self.process.lock().await;
            let pending = proc.reader.pending.clone();
            let writer = &mut proc.writer;
            // Register the pending waiter (inside send_request_and_wait) BEFORE
            // performing the write, so a fast reply from the server isn't dropped
            // by the reader before we're waiting for it.
            send_request_and_wait(id, msg.clone(), &pending, async {
                writer.send(&msg).await.map_err(|e| anyhow::anyhow!("{e}"))
            })
            .await
        };

        if result.is_ok() {
            // Send initialized notification (no response expected)
            let notif = json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {}
            });
            let mut proc = self.process.lock().await;
            let _ = proc.writer.send(&notif).await;
        }

        result
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, String> {
        let id = self.counter.next().await;
        let msg = build_request(id, "tools/list", None);

        let result = {
            let mut proc = self.process.lock().await;
            let pending = proc.reader.pending.clone();
            let writer = &mut proc.writer;
            send_request_and_wait(id, msg.clone(), &pending, async {
                writer.send(&msg).await.map_err(|e| anyhow::anyhow!("{e}"))
            })
            .await
        }?;

        let tools = parse_tools_list(result)?;
        // Update cache
        let mut guard = self.cached_tools.lock().await;
        *guard = tools.clone();
        Ok(tools)
    }

    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        tracing::debug!(
            "[mcp-client] server_id={} tool_call tool_name={}",
            self.server_id,
            tool_name
        );
        let id = self.counter.next().await;
        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });
        let msg = build_request(id, "tools/call", Some(params));

        let mut proc = self.process.lock().await;
        let pending = proc.reader.pending.clone();
        let writer = &mut proc.writer;
        send_request_and_wait(id, msg.clone(), &pending, async {
            writer.send(&msg).await.map_err(|e| anyhow::anyhow!("{e}"))
        })
        .await
    }

    async fn shutdown(&self) {
        tracing::debug!("[mcp-client] shutdown server_id={}", self.server_id);
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "reason": "client shutdown" }
        });
        let mut proc = self.process.lock().await;
        let _ = proc.writer.send(&notif).await;
        let _ = proc.child.kill().await;
    }
}

// ── FakeMcpTransport (test double) ──────────────────────────────────────────

/// An in-memory fake for `McpTransport` usable in unit and E2E tests.
/// Responds to `initialize`, `list_tools`, and `call_tool` without spawning
/// any real process.
pub struct FakeMcpTransport {
    pub tools: Vec<McpTool>,
    /// Canned result for `call_tool`. If `Err`, the call returns that error.
    pub call_result: Result<Value, String>,
}

impl FakeMcpTransport {
    pub fn new(tools: Vec<McpTool>, call_result: Result<Value, String>) -> Arc<Self> {
        Arc::new(Self { tools, call_result })
    }

    pub fn empty() -> Arc<Self> {
        Self::new(Vec::new(), Ok(Value::Null))
    }
}

#[async_trait]
impl McpTransport for FakeMcpTransport {
    async fn initialize(&self) -> Result<Value, String> {
        Ok(json!({
            "protocolVersion": transport::MCP_PROTOCOL_VERSION,
            "capabilities": {}
        }))
    }

    async fn list_tools(&self) -> Result<Vec<McpTool>, String> {
        Ok(self.tools.clone())
    }

    async fn call_tool(&self, _tool_name: &str, _arguments: Value) -> Result<Value, String> {
        self.call_result.clone()
    }

    async fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: Some(format!("Description for {name}")),
            input_schema: json!({ "type": "object" }),
        }
    }

    #[tokio::test]
    async fn fake_initialize_returns_protocol_version() {
        let fake = FakeMcpTransport::empty();
        let result = fake.initialize().await.unwrap();
        assert_eq!(result["protocolVersion"], transport::MCP_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn fake_list_tools_returns_configured_tools() {
        let tools = vec![make_tool("search"), make_tool("write")];
        let fake = FakeMcpTransport::new(tools.clone(), Ok(Value::Null));
        let listed = fake.list_tools().await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "search");
    }

    #[tokio::test]
    async fn fake_call_tool_returns_configured_result() {
        let expected = json!({ "answer": 42 });
        let fake = FakeMcpTransport::new(vec![], Ok(expected.clone()));
        let result = fake.call_tool("any_tool", json!({})).await.unwrap();
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn fake_call_tool_propagates_error() {
        let fake = FakeMcpTransport::new(vec![], Err("tool failed".to_string()));
        let err = fake.call_tool("tool", json!({})).await.unwrap_err();
        assert_eq!(err, "tool failed");
    }

    #[tokio::test]
    async fn fake_shutdown_does_not_panic() {
        let fake = FakeMcpTransport::empty();
        fake.shutdown().await;
    }
}
