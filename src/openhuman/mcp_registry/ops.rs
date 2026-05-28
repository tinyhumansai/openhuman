//! RPC handler implementations for the MCP clients domain.
//!
//! Each function maps 1-to-1 with a `schemas.rs` handler and is testable
//! in isolation; live-process tests live in `tests/json_rpc_e2e.rs`.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::connections;
use super::registry;
use super::store;
use super::types::{CommandKind, ConnStatus, InstalledServer};

// ── registry_search ───────────────────────────────────────────────────────────

pub async fn mcp_clients_registry_search(
    config: &Config,
    query: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<RpcOutcome<Value>, String> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(20);

    tracing::debug!(
        "[mcp-client] registry_search query={:?} page={} page_size={}",
        query,
        page,
        page_size
    );

    let (servers, total_pages) =
        registry::registry_search(config, query.as_deref(), page, page_size)
            .await
            .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::new(
        json!({ "servers": servers, "page": page, "total_pages": total_pages }),
        vec![format!(
            "registry_search returned {} servers",
            servers.len()
        )],
    ))
}

// ── registry_get ──────────────────────────────────────────────────────────────

pub async fn mcp_clients_registry_get(
    config: &Config,
    qualified_name: String,
) -> Result<RpcOutcome<Value>, String> {
    if qualified_name.trim().is_empty() {
        return Err("qualified_name must not be empty".to_string());
    }

    tracing::debug!(
        "[mcp-client] registry_get qualified_name={}",
        qualified_name
    );

    let detail = registry::registry_get(config, qualified_name.trim())
        .await
        .map_err(|e| e.to_string())?;

    // Augment the response with required_env_keys derived from the connection
    // config_schema so the frontend install dialog can build its input form.
    let required_env_keys = collect_required_env_keys(&detail);
    let mut server_value =
        serde_json::to_value(&detail).map_err(|e| format!("serialization error: {e}"))?;
    if let Some(obj) = server_value.as_object_mut() {
        obj.insert(
            "required_env_keys".to_string(),
            serde_json::to_value(&required_env_keys).unwrap_or_else(|_| Value::Array(Vec::new())),
        );
    }

    Ok(RpcOutcome::new(
        json!({ "server": server_value }),
        vec![format!(
            "registry_get ok: {} env_keys={}",
            qualified_name.trim(),
            required_env_keys.len()
        )],
    ))
}

// ── installed_list ────────────────────────────────────────────────────────────

pub async fn mcp_clients_installed_list(config: &Config) -> Result<RpcOutcome<Value>, String> {
    tracing::debug!("[mcp-client] installed_list");
    let installed = store::list_servers(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        json!({ "installed": installed }),
        vec![format!(
            "installed_list returned {} servers",
            installed.len()
        )],
    ))
}

// ── install ───────────────────────────────────────────────────────────────────

pub async fn mcp_clients_install(
    config: &Config,
    qualified_name: String,
    env: HashMap<String, String>,
    config_value: Option<Value>,
) -> Result<RpcOutcome<Value>, String> {
    if qualified_name.trim().is_empty() {
        return Err("qualified_name must not be empty".to_string());
    }

    tracing::debug!(
        "[mcp-client] install qualified_name={} env_keys={:?}",
        qualified_name,
        env.keys().collect::<Vec<_>>()
    );

    // Fetch registry detail to resolve command/args/env_keys
    let detail = registry::registry_get(config, qualified_name.trim())
        .await
        .map_err(|e| format!("Failed to fetch registry detail: {e}"))?;

    // Resolve stdio connection details (prefer published=true, fall back to first)
    let stdio_conn = detail
        .connections
        .iter()
        .filter(|c| c.r#type == "stdio")
        .find(|c| c.published)
        .or_else(|| detail.connections.iter().find(|c| c.r#type == "stdio"));

    // Derive command and args from qualified_name (npm/npx convention)
    let (command_kind, command, args) = resolve_command(qualified_name.trim(), stdio_conn);

    // Derive required env keys from provided map + schema
    let env_keys: Vec<String> = env.keys().cloned().collect();

    let server_id = Uuid::new_v4().to_string();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // The legacy install path only ever picked stdio connections (see the
    // `c.r#type == "stdio"` filter above), so legacy installs continue to
    // be stdio-only. HTTP-remote installs go through the newer
    // `setup_ops::mcp_setup_install_and_connect` setup-agent path, which
    // picks the right transport based on what the registry actually
    // exposes.
    let server = InstalledServer {
        server_id: server_id.clone(),
        qualified_name: qualified_name.trim().to_string(),
        display_name: detail.display_name.clone(),
        description: detail.description.clone(),
        icon_url: detail.icon_url.clone(),
        command_kind,
        command,
        args,
        env_keys,
        config: config_value,
        installed_at: now_ms,
        last_connected_at: None,
        transport: super::types::Transport::Stdio,
    };

    store::insert_server(config, &server).map_err(|e| e.to_string())?;
    store::set_env_values(config, &server_id, &env).map_err(|e| e.to_string())?;

    tracing::debug!(
        "[mcp-client] install ok server_id={} qualified_name={}",
        server_id,
        server.qualified_name
    );

    let _ = publish_global(DomainEvent::McpServerInstalled {
        server_id: server_id.clone(),
        qualified_name: server.qualified_name.clone(),
    });

    Ok(RpcOutcome::new(
        json!({ "server": server }),
        vec![format!("installed server_id={server_id}")],
    ))
}

/// Resolve the launch command from the qualified name and optional registry connection metadata.
pub(super) fn resolve_command(
    qualified_name: &str,
    stdio_conn: Option<&super::types::SmitheryConnection>,
) -> (CommandKind, String, Vec<String>) {
    // Check if the connection has example_config with a command hint
    if let Some(conn) = stdio_conn {
        if let Some(example) = &conn.example_config {
            if let Some(cmd) = example.get("command").and_then(Value::as_str) {
                let args = example
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let kind = if cmd.contains("uvx") || cmd.contains("python") {
                    CommandKind::Python
                } else {
                    CommandKind::Node
                };
                return (kind, cmd.to_string(), args);
            }
        }
    }

    // Default: npx for all packages — both npm-scoped (@org/pkg) and
    // plain smithery-style (owner/name) are launched the same way.
    (
        CommandKind::Node,
        "npx".to_string(),
        vec!["-y".to_string(), qualified_name.to_string()],
    )
}

// ── uninstall ─────────────────────────────────────────────────────────────────

pub async fn mcp_clients_uninstall(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }

    tracing::debug!("[mcp-client] uninstall server_id={}", server_id);

    // Disconnect if currently connected
    connections::disconnect(server_id.trim()).await;

    let removed = store::delete_server(config, server_id.trim()).map_err(|e| e.to_string())?;
    tracing::debug!(
        "[mcp-client] uninstall server_id={} removed={}",
        server_id,
        removed
    );

    Ok(RpcOutcome::new(
        json!({ "server_id": server_id.trim(), "removed": removed }),
        vec![format!("uninstalled server_id={}", server_id.trim())],
    ))
}

// ── connect ────────────────────────────────────────────────────────────────────

pub async fn mcp_clients_connect(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }

    tracing::debug!("[mcp-client] connect rpc server_id={}", server_id);

    let server = store::get_server(config, server_id.trim()).map_err(|e| e.to_string())?;

    let tools = connections::connect(config, &server)
        .await
        .map_err(|e| e.to_string())?;

    let tool_count = tools.len() as u32;

    let _ = publish_global(DomainEvent::McpServerConnected {
        server_id: server_id.trim().to_string(),
        tool_count,
    });

    Ok(RpcOutcome::new(
        json!({
            "server_id": server_id.trim(),
            "status": "connected",
            "tools": tools
        }),
        vec![format!(
            "connected server_id={} tools={}",
            server_id.trim(),
            tool_count
        )],
    ))
}

// ── disconnect ────────────────────────────────────────────────────────────────

pub async fn mcp_clients_disconnect(server_id: String) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }

    tracing::debug!("[mcp-client] disconnect rpc server_id={}", server_id);

    connections::disconnect(server_id.trim()).await;

    let _ = publish_global(DomainEvent::McpServerDisconnected {
        server_id: server_id.trim().to_string(),
        reason: None,
    });

    Ok(RpcOutcome::new(
        json!({ "server_id": server_id.trim(), "status": "disconnected" }),
        vec![format!("disconnected server_id={}", server_id.trim())],
    ))
}

// ── status ─────────────────────────────────────────────────────────────────────

pub async fn mcp_clients_status(config: &Config) -> Result<RpcOutcome<Value>, String> {
    tracing::debug!("[mcp-client] status");
    let statuses: Vec<ConnStatus> = connections::all_status(config).await;
    Ok(RpcOutcome::new(
        json!({ "servers": statuses }),
        vec![format!("status returned {} servers", statuses.len())],
    ))
}

// ── tool_call ─────────────────────────────────────────────────────────────────

pub async fn mcp_clients_tool_call(
    server_id: String,
    tool_name: String,
    arguments: Value,
) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }
    if tool_name.trim().is_empty() {
        return Err("tool_name must not be empty".to_string());
    }

    tracing::debug!(
        "[mcp-client] tool_call server_id={} tool_name={}",
        server_id,
        tool_name
    );

    let start = Instant::now();
    let result = connections::call_tool(server_id.trim(), tool_name.trim(), arguments).await;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let success = result.is_ok();

    let _ = publish_global(DomainEvent::McpClientToolExecuted {
        server_id: server_id.trim().to_string(),
        tool_name: tool_name.trim().to_string(),
        success,
        elapsed_ms,
    });

    match result {
        Ok(value) => Ok(RpcOutcome::new(
            json!({ "result": value, "is_error": false }),
            vec![format!(
                "tool_call ok server_id={} tool={} elapsed_ms={}",
                server_id.trim(),
                tool_name.trim(),
                elapsed_ms
            )],
        )),
        Err(e) => Ok(RpcOutcome::new(
            json!({ "result": e, "is_error": true }),
            vec![format!(
                "tool_call error server_id={} tool={}: {}",
                server_id.trim(),
                tool_name.trim(),
                e
            )],
        )),
    }
}

// ── config_assist ─────────────────────────────────────────────────────────────

pub async fn mcp_clients_config_assist(
    config: &Config,
    qualified_name: String,
    user_message: String,
    history: Option<Vec<super::types::ChatTurn>>,
) -> Result<RpcOutcome<Value>, String> {
    if qualified_name.trim().is_empty() {
        return Err("qualified_name must not be empty".to_string());
    }

    tracing::debug!(
        "[mcp-client] config_assist qualified_name={} message_len={}",
        qualified_name,
        user_message.len()
    );

    // Fetch registry detail to build the system prompt
    let detail = registry::registry_get(config, qualified_name.trim())
        .await
        .map_err(|e| format!("Failed to fetch registry detail: {e}"))?;

    // Collect required env keys from connections (if already known) or from any
    // registered schema in the connection detail.
    let required_env_keys: Vec<String> = collect_required_env_keys(&detail);

    let system_prompt = build_config_assist_system_prompt(
        &detail.display_name,
        qualified_name.trim(),
        &required_env_keys,
    );

    // Build a conversation with the current system prompt + history + new message
    let history = history.unwrap_or_default();

    // Call the agent inference path using the existing infrastructure.
    // We use a simple inline approach: ask the agent to reply in JSON
    // `{ "reply": "...", "suggested_env": { "KEY": "value" } }`.
    let reply_json =
        invoke_config_assist_agent(config, &system_prompt, &history, &user_message).await?;

    let reply = reply_json
        .get("reply")
        .and_then(Value::as_str)
        .unwrap_or("I can help you configure this MCP server. What do you need?")
        .to_string();

    let suggested_env: Option<HashMap<String, String>> = reply_json
        .get("suggested_env")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    Ok(RpcOutcome::new(
        json!({ "reply": reply, "suggested_env": suggested_env }),
        vec!["config_assist replied".to_string()],
    ))
}

fn build_config_assist_system_prompt(
    display_name: &str,
    qualified_name: &str,
    required_env_keys: &[String],
) -> String {
    let keys_list = if required_env_keys.is_empty() {
        "none detected".to_string()
    } else {
        required_env_keys.join(", ")
    };
    format!(
        "You are helping a non-technical user configure an MCP server called `{display_name}` ({qualified_name}). \
         The server requires these env vars: {keys_list}. \
         Walk them through getting each one (where to obtain API keys, etc). \
         If they share values in their message, extract them into the `suggested_env` field. \
         Always respond with a JSON object containing exactly two keys: \
         `reply` (a friendly markdown string explaining what to do next) and \
         `suggested_env` (an object mapping env var names to values, or null if none detected). \
         Do not include any text outside the JSON object."
    )
}

fn collect_required_env_keys(detail: &super::types::SmitheryServerDetail) -> Vec<String> {
    let mut keys = Vec::new();
    for conn in &detail.connections {
        if conn.r#type != "stdio" {
            continue;
        }
        if let Some(schema) = &conn.config_schema {
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for key in props.keys() {
                    if !keys.contains(key) {
                        keys.push(key.clone());
                    }
                }
            }
        }
    }
    keys
}

/// Invoke a lightweight inference call for config_assist.
/// Uses the existing `inference` domain to run a structured-output chat turn.
async fn invoke_config_assist_agent(
    config: &Config,
    system_prompt: &str,
    history: &[super::types::ChatTurn],
    user_message: &str,
) -> Result<Value, String> {
    // Build a simple prompt that asks for JSON output.
    // We delegate to the inference domain if available, otherwise return a fallback.
    let mut full_prompt = String::new();
    for turn in history {
        full_prompt.push_str(&format!("{}: {}\n\n", turn.role, turn.content));
    }
    full_prompt.push_str(&format!("user: {user_message}"));

    tracing::debug!(
        "[mcp-client] config_assist invoke inference prompt_len={}",
        full_prompt.len()
    );

    // Attempt to use the inference infrastructure; fall back to a helpful stub
    // if inference is not configured (common in test environments).
    let api_url = config.api_url.as_deref().unwrap_or("");
    let api_key = config.api_key.as_deref().unwrap_or("");

    if api_url.is_empty() || api_key.is_empty() {
        tracing::debug!("[mcp-client] config_assist no inference config, using stub reply");
        return Ok(json!({
            "reply": "I need to help you configure this MCP server. Please share the required environment variables and I will guide you through the setup.",
            "suggested_env": null
        }));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let messages = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": full_prompt }),
    ];

    let body = json!({
        "model": config.default_model.as_deref().unwrap_or("chat-v1"),
        "messages": messages,
        "temperature": 0.3
    });

    let resp = client
        .post(format!("{api_url}/openai/v1/chat/completions"))
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Inference request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        // Truncate at a Unicode-safe char boundary rather than a raw byte index.
        let preview: String = text.chars().take(200).collect();
        tracing::warn!(
            "[mcp-client] config_assist inference HTTP {}: {}",
            status,
            preview
        );
        return Ok(json!({
            "reply": "I'm currently unable to connect to the AI backend. Please try again shortly.",
            "suggested_env": null
        }));
    }

    let response: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let content = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    // Try to parse the content as JSON; if not, wrap it
    serde_json::from_str::<Value>(&content)
        .or_else(|_| Ok(json!({ "reply": content, "suggested_env": null })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_config_assist_system_prompt_lists_env_keys() {
        let prompt = build_config_assist_system_prompt(
            "Test Server",
            "@test/server",
            &["API_KEY".to_string(), "SECRET".to_string()],
        );
        assert!(prompt.contains("API_KEY"));
        assert!(prompt.contains("SECRET"));
        assert!(prompt.contains("Test Server"));
        assert!(prompt.contains("@test/server"));
    }

    #[test]
    fn build_config_assist_system_prompt_no_keys() {
        let prompt = build_config_assist_system_prompt("My Server", "@my/server", &[]);
        assert!(prompt.contains("none detected"));
    }

    #[test]
    fn collect_required_env_keys_from_schema() {
        use crate::openhuman::mcp_registry::types::{SmitheryConnection, SmitheryServerDetail};
        let detail = SmitheryServerDetail {
            qualified_name: "@test/s".to_string(),
            display_name: "T".to_string(),
            description: None,
            icon_url: None,
            connections: vec![SmitheryConnection {
                r#type: "stdio".to_string(),
                deployment_url: None,
                config_schema: Some(json!({
                    "properties": {
                        "API_KEY": { "type": "string" },
                        "ENDPOINT": { "type": "string" }
                    }
                })),
                example_config: None,
                published: true,
                extra: Default::default(),
            }],
            source: "smithery".to_string(),
            extra: Default::default(),
        };
        let keys = collect_required_env_keys(&detail);
        assert!(keys.contains(&"API_KEY".to_string()));
        assert!(keys.contains(&"ENDPOINT".to_string()));
    }

    #[test]
    fn resolve_command_npm_package() {
        let (kind, cmd, args) = resolve_command("@modelcontextprotocol/server-fs", None);
        assert_eq!(kind, CommandKind::Node);
        assert_eq!(cmd, "npx");
        assert!(args.contains(&"@modelcontextprotocol/server-fs".to_string()));
    }

    #[test]
    fn resolve_command_with_example_config() {
        use crate::openhuman::mcp_registry::types::SmitheryConnection;
        let conn = SmitheryConnection {
            r#type: "stdio".to_string(),
            deployment_url: None,
            config_schema: None,
            example_config: Some(json!({
                "command": "uvx",
                "args": ["--from", "my-pkg", "mcp-server"]
            })),
            published: true,
            extra: Default::default(),
        };
        let (kind, cmd, args) = resolve_command("my-pkg", Some(&conn));
        assert_eq!(kind, CommandKind::Python);
        assert_eq!(cmd, "uvx");
        assert_eq!(args[0], "--from");
    }
}
