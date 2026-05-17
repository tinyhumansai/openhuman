use super::client::{render_tool_result, McpInitializeResult, McpRemoteTool, McpServerToolResult};
use crate::openhuman::config::McpClientIdentityConfig;
use anyhow::Context;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

#[derive(Debug)]
pub struct McpStdioClient {
    command: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<PathBuf>,
    next_id: AtomicI64,
    client_name: String,
    client_title: String,
    client_version: String,
    state: Mutex<Option<StdioSession>>,
}

#[derive(Debug)]
struct StdioSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    initialize: McpInitializeResult,
}

impl McpStdioClient {
    pub fn new(
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<PathBuf>,
        identity: McpClientIdentityConfig,
    ) -> Self {
        Self {
            command,
            args,
            env,
            cwd,
            next_id: AtomicI64::new(1),
            client_name: identity.name,
            client_title: identity.title,
            client_version: identity.version,
            state: Mutex::new(None),
        }
    }

    pub async fn initialize(&self) -> anyhow::Result<McpInitializeResult> {
        let mut state = self.state.lock().await;
        if let Some(session) = state.as_ref() {
            return Ok(session.initialize.clone());
        }

        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(cwd) = self.cwd.as_ref() {
            command.current_dir(cwd);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning MCP stdio server `{}`", self.command))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("stdio server missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("stdio server missing stdout"))?;
        let mut session = StdioSession {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            initialize: McpInitializeResult {
                protocol_version: LATEST_PROTOCOL_VERSION.into(),
                capabilities: json!({}),
                server_info: json!({}),
                instructions: None,
            },
        };

        let response = self
            .send_request_on_session(
                &mut session,
                "initialize",
                json!({
                    "protocolVersion": LATEST_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": self.client_name,
                        "title": self.client_title,
                        "version": self.client_version,
                    }
                }),
            )
            .await?;
        let init: McpInitializeResult =
            serde_json::from_value(response).context("parsing stdio initialize result")?;
        self.send_notification_on_session(&mut session, "notifications/initialized", json!({}))
            .await?;
        session.initialize = init.clone();
        *state = Some(session);
        Ok(init)
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpRemoteTool>> {
        self.initialize().await?;
        let mut state = self.state.lock().await;
        let session = state
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("stdio MCP session not initialized"))?;
        let response = self
            .send_request_on_session(session, "tools/list", json!({}))
            .await?;
        serde_json::from_value(
            response
                .get("tools")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("stdio tools/list response missing `tools`"))?,
        )
        .context("parsing stdio tools/list payload")
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> anyhow::Result<McpServerToolResult> {
        self.initialize().await?;
        let mut state = self.state.lock().await;
        let session = state
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("stdio MCP session not initialized"))?;
        let result = self
            .send_request_on_session(
                session,
                "tools/call",
                json!({
                    "name": name,
                    "arguments": arguments,
                }),
            )
            .await?;
        let rendered = render_tool_result(&result);
        Ok(McpServerToolResult {
            raw_result: result,
            rendered,
        })
    }

    pub async fn close_session(&self) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        if let Some(mut session) = state.take() {
            let _ = session.child.start_kill();
            let _ = session.child.wait().await;
        }
        Ok(())
    }

    async fn send_request_on_session(
        &self,
        session: &mut StdioSession,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        session.stdin.write_all(line.as_bytes()).await?;
        session.stdin.write_all(b"\n").await?;
        session.stdin.flush().await?;

        loop {
            let mut response = String::new();
            let read = session.stdout.read_line(&mut response).await?;
            if read == 0 {
                anyhow::bail!("stdio MCP server closed stdout while waiting for `{method}`");
            }
            let trimmed = response.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
                tracing::debug!(
                    target: "[mcp_client::stdio]",
                    command = %self.command,
                    line = %trimmed,
                    "ignoring non-JSON stdout line from stdio MCP server"
                );
                continue;
            }
            let payload: Value = serde_json::from_str(trimmed)
                .with_context(|| format!("parsing stdio MCP response: {trimmed}"))?;
            if let Some(err) = payload.get("error") {
                anyhow::bail!("MCP stdio error: {err}");
            }
            return payload
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("stdio MCP response missing `result`: {payload}"));
        }
    }

    async fn send_notification_on_session(
        &self,
        session: &mut StdioSession,
        method: &str,
        params: Value,
    ) -> anyhow::Result<()> {
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))?;
        session.stdin.write_all(line.as_bytes()).await?;
        session.stdin.write_all(b"\n").await?;
        session.stdin.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn openhuman_core_bin() -> PathBuf {
        let status = std::process::Command::new("cargo")
            .args(["build", "--quiet", "--bin", "openhuman-core"])
            .status()
            .expect("spawn cargo build");
        assert!(status.success(), "cargo build --bin openhuman-core failed");

        let exe = std::env::current_exe().expect("current_exe");
        let debug_dir = exe
            .parent()
            .and_then(|p| p.parent())
            .expect("target/debug dir");
        let bin = debug_dir.join("openhuman-core");
        assert!(bin.exists(), "expected openhuman-core at {}", bin.display());
        bin
    }

    #[tokio::test]
    async fn stdio_client_talks_to_openhuman_mcp_server() {
        let client = McpStdioClient::new(
            openhuman_core_bin().display().to_string(),
            vec!["mcp".into()],
            Vec::new(),
            Some(std::env::current_dir().unwrap()),
            McpClientIdentityConfig::default(),
        );
        let init = client.initialize().await.expect("initialize");
        assert_eq!(init.protocol_version, LATEST_PROTOCOL_VERSION);
        let tools = client.list_tools().await.expect("list_tools");
        assert!(tools.iter().any(|tool| tool.name == "memory.search"));
        client.close_session().await.expect("close");
    }
}
