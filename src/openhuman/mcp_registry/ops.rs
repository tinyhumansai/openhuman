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

    // Pick the best dialable connection — published stdio > any stdio >
    // published http_remote > any http_remote — using the same picker the
    // setup-agent path uses. Previously this path was stdio-only, so most
    // Smithery listings (HTTP-remote) could not be installed from the manual
    // install dialog at all; now both transports work identically
    // (issue #3039 gap A2).
    let picked = super::setup_ops::pick_connection(&detail.connections).ok_or_else(|| {
        format!(
            "server `{}` exposes neither stdio nor http_remote connections; nothing to install",
            qualified_name.trim()
        )
    })?;
    let (transport, command_kind, command, args) =
        super::setup_ops::build_install_transport(qualified_name.trim(), picked)?;

    // Derive required env keys from provided map + schema
    let env_keys: Vec<String> = env.keys().cloned().collect();

    let server_id = Uuid::new_v4().to_string();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

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
        transport,
        enabled: true,
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

// ── auth detection + browser OAuth ──────────────────────────────────────────────

/// Classify how a server authenticates (`none` / `token` / `oauth`) by probing
/// it — the connect modal renders the matching control. Registry metadata is
/// unreliable, so this is the source of truth.
pub async fn mcp_clients_detect_auth(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }
    let detection = super::oauth::detect(config, server_id.trim()).await?;
    let kind = detection.kind.clone();
    let value = serde_json::to_value(&detection).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        value,
        vec![format!("detect_auth {} -> {}", server_id.trim(), kind)],
    ))
}

/// Begin browser OAuth: discover + dynamic client registration + PKCE, returning
/// the live `/authorize` URL for the frontend to open. The `/oauth/mcp/callback`
/// route completes the exchange + reconnect.
pub async fn mcp_clients_oauth_begin(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }
    let authorize_url = super::oauth::begin(config, server_id.trim()).await?;
    Ok(RpcOutcome::new(
        json!({ "authorize_url": authorize_url }),
        vec![format!("oauth_begin {}", server_id.trim())],
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

    if !server.enabled {
        return Err(format!(
            "server_id={} is disabled; enable it via mcp_clients_set_enabled before connecting",
            server_id.trim()
        ));
    }

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

// ── set_enabled ────────────────────────────────────────────────────────────────

/// Flip the `enabled` flag on an installed server.
///
/// - `enabled=false`: persist the flip, then disconnect any live session so
///   the server's tools immediately disappear from the agent's surface. The
///   install row and env values are kept intact so re-enabling later does
///   not require re-entering credentials.
/// - `enabled=true`: persist the flip. The server is NOT auto-connected here
///   — the user calls `connect` explicitly. This keeps "enabled" purely a
///   persistent setting and "connected" purely a live-session state.
pub async fn mcp_clients_set_enabled(
    config: &Config,
    server_id: String,
    enabled: bool,
) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }
    let server_id = server_id.trim().to_string();

    tracing::debug!(
        "[mcp-client] set_enabled server_id={} enabled={}",
        server_id,
        enabled
    );

    // Existence check produces a clear error before we mutate.
    let _existing = store::get_server(config, &server_id).map_err(|e| e.to_string())?;
    store::update_enabled(config, &server_id, enabled).map_err(|e| e.to_string())?;

    if !enabled {
        connections::disconnect(&server_id).await;
        connections::clear_last_error(&server_id).await;
        let _ = publish_global(DomainEvent::McpServerDisconnected {
            server_id: server_id.clone(),
            reason: Some("disabled".to_string()),
        });
    }

    Ok(RpcOutcome::new(
        json!({ "server_id": server_id, "enabled": enabled }),
        vec![format!(
            "set_enabled server_id={server_id} enabled={enabled}"
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

// ── update_env ───────────────────────────────────────────────────────────────

/// Replace the stored env values for an already-installed server and reconnect
/// so the new credentials take effect immediately (issue #3039 gap A3 — API-key
/// rotation / reconfigure without uninstall+reinstall).
///
/// Flow: persist env → disconnect (drop the stale session) → reload the install
/// record → reconnect with the fresh env. The reconnect reuses the transport
/// dispatch in [`connections::connect`], so this works for both stdio and
/// HTTP-remote installs. Values are never logged — only the key names.
///
/// A failed reconnect does **not** roll back the persisted env (matching
/// `mcp_setup_install_and_connect`): the user fixed a value, we keep it, and the
/// error is surfaced so they can retry `mcp_clients_connect`.
pub async fn mcp_clients_update_env(
    config: &Config,
    server_id: String,
    env: HashMap<String, String>,
) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }
    let server_id = server_id.trim();

    tracing::debug!(
        "[mcp-client] update_env server_id={} env_keys={:?}",
        server_id,
        env.keys().collect::<Vec<_>>()
    );

    // Merge the supplied values over any already-stored env, THEN persist —
    // `set_env_values` replaces the value table wholesale, so a partial update
    // (e.g. the connect modal sending only the one field the user just typed,
    // with no way to display the other stored secrets) would silently erase
    // the rest. Merging preserves keys the caller didn't send; supplied values
    // win on collision. Callers that send every key (the reconfigure form,
    // which requires all fields) are unaffected — for them merged == supplied.
    let mut merged = store::load_env_values(config, server_id).unwrap_or_default();
    merged.extend(env);
    // Persist first so the new values survive even if the reconnect fails.
    store::set_env_values(config, server_id, &merged).map_err(|e| e.to_string())?;

    // Drop any live session so the reconnect picks up the new env.
    connections::disconnect(server_id).await;
    let _ = publish_global(DomainEvent::McpServerDisconnected {
        server_id: server_id.to_string(),
        reason: Some("env reconfigured".to_string()),
    });

    let mut server = store::get_server(config, server_id).map_err(|e| e.to_string())?;

    // Keep the install record's `env_keys` list in sync with the full merged
    // value set we just wrote (not just the keys supplied this call), so the
    // key-name list shown in the UI (and returned below) reflects every stored
    // key — including the ones a partial update preserved.
    let mut new_keys: Vec<String> = merged.keys().cloned().collect();
    new_keys.sort();
    if server.env_keys != new_keys {
        server.env_keys = new_keys;
        store::update_server_env_keys(config, server_id, &server.env_keys)
            .map_err(|e| e.to_string())?;
    }

    // A disabled server must not be auto-reconnected even when its env is
    // reconfigured — `enabled` is the user-visible "should this be live" gate
    // and the same disabled rule that blocks `mcp_clients_connect` applies
    // here. The new values are already persisted so a later `set_enabled(true)`
    // + `connect` round-trip will pick them up.
    if !server.enabled {
        return Ok(RpcOutcome::new(
            json!({
                "server_id": server_id,
                "status": "disabled",
                "env_keys": server.env_keys,
            }),
            vec![format!(
                "update_env persisted env for server_id={server_id} but did not reconnect: server is disabled"
            )],
        ));
    }

    match connections::connect(config, &server).await {
        Ok(tools) => {
            let tool_count = tools.len() as u32;
            let _ = publish_global(DomainEvent::McpServerConnected {
                server_id: server_id.to_string(),
                tool_count,
            });
            Ok(RpcOutcome::new(
                json!({
                    "server_id": server_id,
                    "status": "connected",
                    "env_keys": server.env_keys,
                    "tools": tools,
                }),
                vec![format!(
                    "update_env reconnected server_id={server_id} tools={tool_count}"
                )],
            ))
        }
        Err(err) => Ok(RpcOutcome::new(
            json!({
                "server_id": server_id,
                "status": "disconnected",
                "env_keys": server.env_keys,
                "error": err.to_string(),
            }),
            vec![format!(
                "update_env persisted env for server_id={server_id} but reconnect failed: {err}"
            )],
        )),
    }
}

// ── registry settings ──────────────────────────────────────────────────────

/// Build the non-secret registry-settings snapshot: booleans reporting whether
/// each credential is set (from config OR env) plus the user-configured
/// official-registry base URL. Secret *values* are never included.
fn registry_settings_snapshot(config: &Config) -> Value {
    json!({
        "smithery_api_key_set":
            super::registries::smithery::smithery_api_key(config).is_some(),
        "mcp_official_token_set":
            super::registries::mcp_official::auth_token(config).is_some(),
        "mcp_official_base":
            config
                .mcp_client
                .registry_auth
                .mcp_official_base
                .clone()
                .filter(|s| !s.trim().is_empty()),
    })
}

/// Report which registry credentials are configured. NEVER returns secret
/// values — only `*_set` booleans + the (non-secret) base URL override
/// (issue #3039 gap A6).
pub async fn mcp_clients_registry_settings_get(
    config: &Config,
) -> Result<RpcOutcome<Value>, String> {
    tracing::debug!("[mcp-client] registry_settings_get");
    Ok(RpcOutcome::new(
        registry_settings_snapshot(config),
        vec!["registry_settings_get".to_string()],
    ))
}

/// Persist registry credentials to config (issue #3039 gap A6).
///
/// Per-field semantics: `None` leaves the stored value unchanged; `Some(s)`
/// sets it, where an empty/whitespace string clears the value (falling back to
/// the env var, if any). Secrets are write-only — the response is the same
/// non-secret snapshot as the getter, never the values just written.
pub async fn mcp_clients_registry_settings_set(
    config: &mut Config,
    smithery_api_key: Option<String>,
    mcp_official_base: Option<String>,
    mcp_official_token: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    fn apply(field: &mut Option<String>, update: Option<String>) {
        if let Some(value) = update {
            let trimmed = value.trim();
            *field = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
    }

    tracing::debug!(
        "[mcp-client] registry_settings_set smithery_key_present={} official_token_present={} official_base_present={}",
        smithery_api_key.is_some(),
        mcp_official_token.is_some(),
        mcp_official_base.is_some()
    );

    let auth = &mut config.mcp_client.registry_auth;
    apply(&mut auth.smithery_api_key, smithery_api_key);
    apply(&mut auth.mcp_official_base, mcp_official_base);
    apply(&mut auth.mcp_official_token, mcp_official_token);

    config.save().await.map_err(|e| e.to_string())?;

    Ok(RpcOutcome::new(
        registry_settings_snapshot(config),
        vec!["registry_settings_set saved".to_string()],
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

// ── list_tools ──────────────────────────────────────────────────────────────

/// List the tools (name + description + input schema) advertised by one
/// already-connected server. This is the agent's discovery primitive: it
/// reads the live snapshot without re-handshaking (unlike `connect`). When
/// the server is not connected, returns an error hint to connect first.
pub async fn mcp_clients_list_tools(server_id: String) -> Result<RpcOutcome<Value>, String> {
    if server_id.trim().is_empty() {
        return Err("server_id must not be empty".to_string());
    }

    tracing::debug!("[mcp-client] list_tools server_id={}", server_id);

    match connections::tools_for(server_id.trim()).await {
        Some(tools) => {
            let count = tools.len();
            Ok(RpcOutcome::new(
                json!({ "server_id": server_id.trim(), "tools": tools }),
                vec![format!(
                    "list_tools server_id={} returned {} tools",
                    server_id.trim(),
                    count
                )],
            ))
        }
        None => Err(format!(
            "server_id={} is not connected; connect it first via mcp_clients_connect",
            server_id.trim()
        )),
    }
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
        // Collect required inputs from every connection type. stdio inputs are
        // process env vars; http / http_remote inputs are request headers
        // (e.g. `Authorization`) declared via the registry's `remotes[].headers`
        // — both are user-supplied secrets the install form must render.
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
    // The legacy JSON-asking system prompt is intentionally unused: the agent
    // turn returns its text verbatim, so we want natural markdown, not a JSON
    // envelope. Server context comes through `user_message`.
    _system_prompt: &str,
    history: &[super::types::ChatTurn],
    user_message: &str,
) -> Result<Value, String> {
    // Run a real agent turn (not a bare completion) so the model can use
    // `web_search` / `web_fetch` / `curl` to look up the provider's actual docs
    // and give accurate, current token-acquisition steps instead of guessing
    // from training memory. The research directive + server context go in the
    // message; the default agent already carries the web tools (always
    // registered), gated by the usual SecurityPolicy.
    let mut message = String::new();
    message.push_str(
        "You are an MCP setup helper. Use web_search and web_fetch/curl to look up the \
         provider's OFFICIAL documentation, then tell the user exactly how to obtain the \
         credential needed to connect this MCP server: where to sign up / log in, where to \
         generate the API key or token, which scopes/permissions to enable, and the exact \
         header name and value format to paste. Reply with concise numbered steps and cite \
         the source URL. Do not invent URLs — verify them with the tools. Respond in plain \
         markdown prose, NOT JSON and with no wrapping object.\n\n",
    );
    for turn in history {
        message.push_str(&format!("{}: {}\n", turn.role, turn.content));
    }
    message.push_str(&format!("user: {user_message}"));

    tracing::debug!(
        "[mcp-client] config_assist running agent turn (web tools) prompt_len={}",
        message.len()
    );

    let mut agent = match crate::openhuman::agent::Agent::from_config(config) {
        Ok(a) => a,
        Err(e) => {
            return Ok(json!({
                "reply": format!(
                    "Couldn't start the assistant: {e}. Make sure AI/inference is configured (Settings → AI)."
                ),
                "suggested_env": null
            }));
        }
    };
    // Scope this docs helper to web-research tools only. `from_config` builds
    // the full default agent surface (filesystem, shell, MCP, browser, …), but
    // a credential-help turn must not be able to pivot into unrelated local
    // capabilities — it only needs to read the provider's public docs (#3648).
    agent.set_visible_tool_names(
        ["web_search_tool", "web_fetch", "curl"]
            .into_iter()
            .map(String::from)
            .collect(),
    );

    // Trusted desktop-initiated turn — label as CLI so the approval gate doesn't
    // fail closed on an unlabelled call site (mirrors `agent_chat`).
    let reply_result = crate::openhuman::agent::turn_origin::with_origin(
        crate::openhuman::agent::turn_origin::AgentTurnOrigin::Cli,
        agent.run_single(&message),
    )
    .await;

    match reply_result {
        Ok(reply) => Ok(json!({ "reply": reply, "suggested_env": null })),
        Err(e) => Ok(json!({
            "reply": format!(
                "I couldn't research that right now: {e}. Make sure AI/inference is configured (Settings → AI)."
            ),
            "suggested_env": null
        })),
    }
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
