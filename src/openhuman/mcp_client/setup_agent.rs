//! One-shot MCP server setup: search, resolve, test, and report.
//!
//! This module provides a high-level [`setup_from_registry`] function that
//! given a server name (or search query), resolves its install command from
//! the official MCP registry, spawns a scratch stdio session to verify the
//! server responds, and returns a [`SetupResult`] with everything needed
//! to persist the install.
//!
//! It is intentionally transport-only — no persistence, no event-bus, no
//! RPC surface. The sibling `mcp_registry` module consumes this for its
//! full lifecycle management; agent tools and the UI can also call it
//! directly for a "dry-run" test before committing an install.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::McpStdioClient;
use crate::openhuman::config::McpClientIdentityConfig;

/// Input parameters for setting up an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupRequest {
    /// The command to launch (e.g. `npx`, `uvx`, `node`, `python`).
    pub command: String,
    /// Arguments passed to the command (e.g. `["-y", "@scope/pkg"]`).
    pub args: Vec<String>,
    /// Environment variables to inject into the subprocess.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional working directory for the subprocess.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

/// Result of a successful setup test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupResult {
    /// Server name reported in the `initialize` handshake.
    pub server_name: String,
    /// Server version reported in the `initialize` handshake.
    pub server_version: String,
    /// Protocol version negotiated.
    pub protocol_version: String,
    /// Tools the server exposes.
    pub tools: Vec<SetupTool>,
}

/// A tool discovered during setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupTool {
    pub name: String,
    pub description: Option<String>,
}

/// Spawn a scratch MCP stdio session, perform the `initialize` handshake,
/// list tools, and tear down. Returns the discovered server info and tools
/// on success.
///
/// This is a side-effect-free probe — nothing is persisted or registered.
pub async fn test_connection(
    req: &SetupRequest,
    identity: McpClientIdentityConfig,
) -> Result<SetupResult> {
    let env: Vec<(String, String)> = req
        .env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    tracing::debug!(
        "[mcp-setup-agent] test_connection command={} args={:?}",
        req.command,
        req.args
    );

    let client = McpStdioClient::new(
        req.command.clone(),
        req.args.clone(),
        env,
        req.cwd.clone(),
        identity,
    );

    let init = match client.initialize().await {
        Ok(init) => init,
        Err(err) => {
            let _ = client.close_session().await;
            return Err(err).context("MCP server failed to initialize");
        }
    };

    let tools = match client.list_tools().await {
        Ok(tools) => tools,
        Err(err) => {
            let _ = client.close_session().await;
            return Err(err).context("MCP server failed to list tools");
        }
    };

    let _ = client.close_session().await;

    let server_name = init
        .server_info
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let server_version = init
        .server_info
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("0.0.0")
        .to_string();

    let setup_tools: Vec<SetupTool> = tools
        .into_iter()
        .map(|t| SetupTool {
            name: t.name,
            description: t.description,
        })
        .collect();

    tracing::info!(
        "[mcp-setup-agent] test_connection ok server={} version={} tools={}",
        server_name,
        server_version,
        setup_tools.len()
    );

    Ok(SetupResult {
        server_name,
        server_version,
        protocol_version: init.protocol_version,
        tools: setup_tools,
    })
}

/// Resolve the install command for a package from the official MCP registry
/// response shape. Given `registry_type` and `identifier`, returns the
/// appropriate [`SetupRequest`].
///
/// Supported registry types:
/// - `"npm"` → `npx -y <identifier>`
/// - `"pypi"` → `uvx <identifier>`
/// - Other → attempts `<identifier>` as a direct binary
pub fn resolve_setup_request(
    registry_type: &str,
    identifier: &str,
    runtime_hint: Option<&str>,
    runtime_args: &[String],
    env: HashMap<String, String>,
) -> SetupRequest {
    let (command, mut args) = match registry_type {
        "pypi" => {
            let cmd = runtime_hint.unwrap_or("uvx").to_string();
            (cmd, Vec::new())
        }
        "npm" => {
            let cmd = runtime_hint.unwrap_or("npx").to_string();
            let default_args = if runtime_args.is_empty() {
                vec!["-y".to_string()]
            } else {
                Vec::new()
            };
            (cmd, default_args)
        }
        _ => {
            let cmd = runtime_hint
                .map(String::from)
                .unwrap_or_else(|| identifier.to_string());
            (cmd, Vec::new())
        }
    };

    for ra in runtime_args {
        args.push(ra.clone());
    }

    if registry_type == "npm" || registry_type == "pypi" {
        args.push(identifier.to_string());
    }

    SetupRequest {
        command,
        args,
        env,
        cwd: None,
    }
}

/// Parse the official MCP registry server detail JSON and extract the best
/// package-based [`SetupRequest`]. Prefers npm, falls back to pypi, then
/// any other registryType.
pub fn setup_request_from_registry_detail(
    detail: &Value,
    env: HashMap<String, String>,
) -> Option<SetupRequest> {
    let packages = detail.get("packages").and_then(Value::as_array)?;

    let pick = packages
        .iter()
        .find(|p| p.get("registryType").and_then(Value::as_str) == Some("npm"))
        .or_else(|| {
            packages
                .iter()
                .find(|p| p.get("registryType").and_then(Value::as_str) == Some("pypi"))
        })
        .or_else(|| packages.first())?;

    let registry_type = pick
        .get("registryType")
        .and_then(Value::as_str)
        .unwrap_or("npm");
    let identifier = pick.get("identifier").and_then(Value::as_str)?;
    let runtime_hint = pick.get("runtimeHint").and_then(Value::as_str);
    let runtime_args: Vec<String> = pick
        .get("runtimeArguments")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("value").and_then(Value::as_str))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    Some(resolve_setup_request(
        registry_type,
        identifier,
        runtime_hint,
        &runtime_args,
        env,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolve_npm_package_with_runtime_args() {
        let req = resolve_setup_request(
            "npm",
            "@scope/my-server",
            Some("npx"),
            &["-y".into()],
            HashMap::new(),
        );
        assert_eq!(req.command, "npx");
        assert_eq!(req.args, vec!["-y", "@scope/my-server"]);
    }

    #[test]
    fn resolve_npm_package_default_args() {
        let req = resolve_setup_request("npm", "@scope/my-server", None, &[], HashMap::new());
        assert_eq!(req.command, "npx");
        assert_eq!(req.args, vec!["-y", "@scope/my-server"]);
    }

    #[test]
    fn resolve_pypi_package() {
        let req = resolve_setup_request("pypi", "mcp-server-time", None, &[], HashMap::new());
        assert_eq!(req.command, "uvx");
        assert_eq!(req.args, vec!["mcp-server-time"]);
    }

    #[test]
    fn resolve_unknown_registry_uses_identifier() {
        let req = resolve_setup_request("docker", "my-image", None, &[], HashMap::new());
        assert_eq!(req.command, "my-image");
        assert!(req.args.is_empty());
    }

    #[test]
    fn setup_request_from_detail_picks_npm_over_pypi() {
        let detail = json!({
            "packages": [
                { "registryType": "pypi", "identifier": "py-server" },
                { "registryType": "npm", "identifier": "@org/node-server", "runtimeHint": "npx", "runtimeArguments": [{"value": "-y"}] },
            ]
        });
        let req = setup_request_from_registry_detail(&detail, HashMap::new()).unwrap();
        assert_eq!(req.command, "npx");
        // runtime_args=["-y"] provided, so no default -y added; then identifier appended
        assert_eq!(req.args, vec!["-y", "@org/node-server"]);
    }

    #[test]
    fn setup_request_from_detail_falls_back_to_pypi() {
        let detail = json!({
            "packages": [
                { "registryType": "pypi", "identifier": "my-mcp" },
            ]
        });
        let req = setup_request_from_registry_detail(&detail, HashMap::new()).unwrap();
        assert_eq!(req.command, "uvx");
        assert_eq!(req.args, vec!["my-mcp"]);
    }

    #[test]
    fn setup_request_with_env() {
        let mut env = HashMap::new();
        env.insert("API_KEY".into(), "secret123".into());
        let req = resolve_setup_request("npm", "my-server", None, &[], env);
        assert_eq!(req.env.get("API_KEY").unwrap(), "secret123");
    }
}
