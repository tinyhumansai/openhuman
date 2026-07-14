use super::client::{render_tool_result, McpInitializeResult, McpRemoteTool, McpServerToolResult};
use super::spawn_env;
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

        // Desktop GUI launches inherit a stripped PATH (e.g. macOS Finder/launchd
        // gives `/usr/bin:/bin:/usr/sbin:/sbin`), so `npx`/`uvx`-based servers
        // can't be found. Rebuild the user's real shell PATH before spawning.
        let resolved_path = spawn_env::spawn_path().await;

        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(cwd) = self.cwd.as_ref() {
            command.current_dir(cwd);
        }
        // Apply the reconstructed PATH first; config-provided env is applied
        // afterwards so an explicit `PATH` override from the server config wins.
        command.env("PATH", &resolved_path);
        for (key, value) in &self.env {
            command.env(key, value);
        }

        // Resolve the command up front so a missing Node/uv runtime fails with
        // actionable guidance instead of a bare ENOENT from `spawn`.
        let effective_path = self
            .env
            .iter()
            .rev()
            .find(|(key, _)| key == "PATH")
            .map(|(_, value)| value.as_str())
            .unwrap_or(resolved_path.as_str());
        if spawn_env::locate_command(&self.command, effective_path, self.cwd.as_deref()).is_none() {
            tracing::warn!(
                target: "[mcp_client::stdio]",
                command = %self.command,
                "stdio MCP command not found on resolved PATH"
            );
            anyhow::bail!(spawn_env::missing_command_error(&self.command));
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

    #[tokio::test]
    async fn initialize_missing_command_returns_actionable_error() {
        let client = McpStdioClient::new(
            "openhuman-nonexistent-binary-zzz".to_string(),
            Vec::new(),
            Vec::new(),
            None,
            McpClientIdentityConfig::default(),
        );
        let err = client
            .initialize()
            .await
            .expect_err("spawn of a missing command must fail");
        assert!(
            err.to_string().contains("was not found"),
            "expected actionable error, got: {err}"
        );
    }

    #[tokio::test]
    async fn initialize_missing_npx_mentions_node() {
        // A config PATH override with no npx forces the missing-runtime branch
        // deterministically, independent of the host's real PATH.
        let client = McpStdioClient::new(
            "npx".to_string(),
            Vec::new(),
            vec![("PATH".to_string(), "/openhuman/does-not-exist".to_string())],
            None,
            McpClientIdentityConfig::default(),
        );
        let err = client
            .initialize()
            .await
            .expect_err("missing npx must fail");
        assert!(
            err.to_string().contains("Node.js"),
            "expected Node.js guidance, got: {err}"
        );
    }
}
