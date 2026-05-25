//! RPC handlers for the MCP setup agent. See `docs/MCP_SETUP_AGENT.md`.
//!
//! These handlers form the agent-facing tool surface:
//!
//! - `mcp_setup_search` / `mcp_setup_get` — thin wrappers over
//!   [`super::registry`] so the agent browses upstream registries.
//! - `mcp_setup_request_secret` — block on a fresh ref until the UI
//!   submits a value.
//! - `mcp_setup_submit_secret` — UI-side fulfillment.
//! - `mcp_setup_test_connection` — spawn a candidate subprocess in a
//!   scratch workspace, list its tools, tear it down. No persistence.
//! - `mcp_setup_install_and_connect` — commit: persist install + env,
//!   call [`super::connections::connect`].
//!
//! Raw secret values flow only through `submit_secret` and the
//! just-in-time resolve inside `test_connection` / `install_and_connect`.
//! They are never echoed in responses or logged.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::mcp_client::McpStdioClient;
use crate::rpc::RpcOutcome;

use super::ops::resolve_command;
use super::setup::{self, SecretRef};
use super::types::{CommandKind, InstalledServer};
use super::{connections, registry, store};

// ── search ───────────────────────────────────────────────────────────────────

pub async fn mcp_setup_search(
    config: &Config,
    query: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<RpcOutcome<Value>, String> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(20);
    let (servers, total_pages) =
        registry::registry_search(config, query.as_deref(), page, page_size)
            .await
            .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        json!({ "servers": servers, "page": page, "total_pages": total_pages }),
        vec![format!("setup_search returned {} servers", servers.len())],
    ))
}

// ── get ──────────────────────────────────────────────────────────────────────

pub async fn mcp_setup_get(
    config: &Config,
    qualified_name: String,
) -> Result<RpcOutcome<Value>, String> {
    let q = qualified_name.trim();
    if q.is_empty() {
        return Err("qualified_name must not be empty".to_string());
    }
    let detail = registry::registry_get(config, q)
        .await
        .map_err(|e| e.to_string())?;
    let required_env_keys = collect_required_env_keys(&detail);
    let mut value = serde_json::to_value(&detail).map_err(|e| format!("ser: {e}"))?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("required_env_keys".into(), json!(required_env_keys));
    }
    Ok(RpcOutcome::new(
        json!({ "server": value }),
        vec![format!("setup_get ok qualified_name={q}")],
    ))
}

// ── request_secret ───────────────────────────────────────────────────────────

pub async fn mcp_setup_request_secret(
    key_name: String,
    prompt: String,
) -> Result<RpcOutcome<Value>, String> {
    let key_name = key_name.trim().to_string();
    let prompt = prompt.trim().to_string();
    if key_name.is_empty() {
        return Err("key_name must not be empty".to_string());
    }
    if prompt.is_empty() {
        return Err("prompt must not be empty".to_string());
    }

    let (r, rx) = setup::mint_request(&key_name).await;

    let _ = publish_global(DomainEvent::McpSetupSecretRequested {
        ref_id: r.as_str().to_string(),
        key_name: key_name.clone(),
        prompt: prompt.clone(),
    });
    tracing::info!(
        "[mcp-setup] request_secret ref={} key_name={} (awaiting UI submit)",
        r.as_str(),
        key_name
    );

    setup::await_fulfillment(&r, rx)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!("[mcp-setup] request_secret fulfilled ref={}", r.as_str());
    Ok(RpcOutcome::new(
        json!({ "ref": r.as_str(), "key_name": key_name }),
        vec![format!("collected secret for key={key_name}")],
    ))
}

// ── submit_secret (UI side) ──────────────────────────────────────────────────

pub async fn mcp_setup_submit_secret(
    ref_id: String,
    value: String,
) -> Result<RpcOutcome<Value>, String> {
    let r = SecretRef::parse(&ref_id).ok_or_else(|| format!("invalid ref_id `{ref_id}`"))?;
    let ok = setup::fulfill(&r, value).await;
    if !ok {
        return Err(format!("ref {} unknown or already submitted", r.as_str()));
    }
    Ok(RpcOutcome::new(
        json!({ "ref": r.as_str(), "fulfilled": true }),
        vec![format!("submitted secret for ref={}", r.as_str())],
    ))
}

// ── test_connection ──────────────────────────────────────────────────────────

pub async fn mcp_setup_test_connection(
    config: &Config,
    qualified_name: String,
    env_refs: HashMap<String, String>,
) -> Result<RpcOutcome<Value>, String> {
    let q = qualified_name.trim();
    if q.is_empty() {
        return Err("qualified_name must not be empty".to_string());
    }

    let parsed_refs = parse_ref_map(env_refs)?;
    let env = setup::resolve_refs(&parsed_refs)
        .await
        .map_err(|e| e.to_string())?;

    let detail = registry::registry_get(config, q)
        .await
        .map_err(|e| e.to_string())?;
    let stdio_conn = detail
        .connections
        .iter()
        .filter(|c| c.r#type == "stdio")
        .find(|c| c.published)
        .or_else(|| detail.connections.iter().find(|c| c.r#type == "stdio"));
    let (_kind, command, args) = resolve_command(q, stdio_conn);

    let identity = config.mcp_client.client_identity.clone();
    let cwd: Option<PathBuf> = None;
    let client = McpStdioClient::new(command.clone(), args.clone(), env, cwd, identity);

    // Scratch subprocess — initialise + list_tools, then close. Nothing
    // persisted. Errors bubble up so the agent can show them to the user.
    if let Err(err) = client.initialize().await {
        return Ok(RpcOutcome::new(
            json!({ "ok": false, "error": err.to_string() }),
            vec![format!("test_connection failed for {q}: {err}")],
        ));
    }
    let tools = match client.list_tools().await {
        Ok(t) => t,
        Err(err) => {
            let _ = client.close_session().await;
            return Ok(RpcOutcome::new(
                json!({ "ok": false, "error": err.to_string() }),
                vec![format!("test_connection list_tools failed for {q}: {err}")],
            ));
        }
    };
    let _ = client.close_session().await;

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    Ok(RpcOutcome::new(
        json!({ "ok": true, "tools": tools }),
        vec![format!(
            "test_connection ok for {q}: {} tools ({:?})",
            tools.len(),
            names
        )],
    ))
}

// ── install_and_connect ──────────────────────────────────────────────────────

pub async fn mcp_setup_install_and_connect(
    config: &Config,
    qualified_name: String,
    env_refs: HashMap<String, String>,
) -> Result<RpcOutcome<Value>, String> {
    let q = qualified_name.trim();
    if q.is_empty() {
        return Err("qualified_name must not be empty".to_string());
    }

    let parsed_refs = parse_ref_map(env_refs)?;

    let detail = registry::registry_get(config, q)
        .await
        .map_err(|e| e.to_string())?;
    let stdio_conn = detail
        .connections
        .iter()
        .filter(|c| c.r#type == "stdio")
        .find(|c| c.published)
        .or_else(|| detail.connections.iter().find(|c| c.r#type == "stdio"));
    let (command_kind, command, args) = resolve_command(q, stdio_conn);

    // Consume refs only after `registry_get` succeeds — that way a
    // misconfigured server name doesn't burn the user's collected
    // secrets.
    let env_pairs = setup::consume_refs(&parsed_refs)
        .await
        .map_err(|e| e.to_string())?;
    let env_map: HashMap<String, String> = env_pairs.into_iter().collect();

    let server_id = Uuid::new_v4().to_string();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let env_keys: Vec<String> = env_map.keys().cloned().collect();

    let server = InstalledServer {
        server_id: server_id.clone(),
        qualified_name: q.to_string(),
        display_name: detail.display_name.clone(),
        description: detail.description.clone(),
        icon_url: detail.icon_url.clone(),
        command_kind,
        command,
        args,
        env_keys,
        config: None,
        installed_at: now_ms,
        last_connected_at: None,
    };

    store::insert_server(config, &server).map_err(|e| e.to_string())?;
    store::set_env_values(config, &server_id, &env_map).map_err(|e| e.to_string())?;

    let _ = publish_global(DomainEvent::McpServerInstalled {
        server_id: server_id.clone(),
        qualified_name: server.qualified_name.clone(),
    });

    // Connect immediately so the agent gets the tool list in the same
    // response. A connect failure does not roll back the install — the
    // user can retry via `mcp_clients_connect` later.
    match connections::connect(config, &server).await {
        Ok(tools) => Ok(RpcOutcome::new(
            json!({
                "server_id": server_id,
                "status": "connected",
                "tools": tools,
            }),
            vec![format!(
                "install_and_connect ok server_id={server_id} tools={}",
                tools.len()
            )],
        )),
        Err(err) => Ok(RpcOutcome::new(
            json!({
                "server_id": server_id,
                "status": "installed_disconnected",
                "error": err.to_string(),
            }),
            vec![format!(
                "install_and_connect installed server_id={server_id} \
                 but connect failed: {err}"
            )],
        )),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn parse_ref_map(raw: HashMap<String, String>) -> Result<HashMap<String, SecretRef>, String> {
    let mut out = HashMap::with_capacity(raw.len());
    for (k, v) in raw {
        let r = SecretRef::parse(&v)
            .ok_or_else(|| format!("env_refs[{k}] is not a valid secret ref"))?;
        out.insert(k, r);
    }
    Ok(out)
}

/// Best-effort scan of a Smithery `config_schema` for required env keys.
/// Mirrors the legacy helper in `ops.rs` so the setup agent does not
/// depend on its private wiring.
fn collect_required_env_keys(detail: &super::types::SmitheryServerDetail) -> Vec<String> {
    let mut keys = Vec::new();
    for conn in &detail.connections {
        if conn.r#type != "stdio" {
            continue;
        }
        let Some(schema) = conn.config_schema.as_ref() else {
            continue;
        };
        let Some(props) = schema.get("properties").and_then(Value::as_object) else {
            continue;
        };
        for k in props.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    keys
}

// Compile-time anchor so a missing CommandKind import surfaces here, not
// at the call site.
#[allow(dead_code)]
const _: Option<CommandKind> = None;
