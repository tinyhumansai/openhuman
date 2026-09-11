//! RPC handlers for the guided setup flow (`mcp_setup`).
//!
//! Every function keeps the signature and the JSON shape it had before the
//! client moved to `tinymcp`, and delegates to the service [`super::super::host`]
//! holds.
//!
//! # The secret handles have not changed shape
//!
//! The flow is still: mint an opaque `secret://…` handle, prompt the user out
//! of band, and resolve the handle inside the operation that needs the value.
//! The raw value never crosses the model-facing surface. `tinymcp` owns the
//! vault; this layer keeps the part that is this application's — publishing the
//! event that makes the prompt appear, and waiting for the answer.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use tinymcp::SecretRef;

use super::helpers::{encode, inject_required_env_keys, require, resolve};
use super::types::{ConnStatus, ServerStatus};

/// Reads a map of credential names to handles.
fn parse_handles(raw: HashMap<String, String>) -> Result<HashMap<String, SecretRef>, String> {
    raw.into_iter()
        .map(|(name, handle)| {
            SecretRef::parse(&handle)
                .map(|parsed| (name, parsed))
                .ok_or_else(|| format!("invalid ref_id `{handle}`"))
        })
        .collect()
}

/// Renders a detail record with the credential names an install would need.
fn detail_payload(
    detail: &tinymcp_bus::RegistryServerDetail,
    required_env_keys: &[String],
) -> Result<Value, String> {
    let mut value = encode(detail)?;
    inject_required_env_keys(&mut value, required_env_keys);
    Ok(value)
}

// ── search ───────────────────────────────────────────────────────────────────

pub async fn mcp_setup_search(
    config: &Config,
    query: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<RpcOutcome<Value>, String> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(20);

    let found = resolve(config)?
        .dynamic()
        .registry_search(query.as_deref(), page, page_size)
        .await
        .map_err(|error| error.to_string())?;

    let count = found.servers.len();
    Ok(RpcOutcome::new(
        json!({
            "servers": found.servers,
            "page": found.page,
            "total_pages": found.total_pages,
        }),
        vec![format!("setup_search returned {count} servers")],
    ))
}

// ── get ──────────────────────────────────────────────────────────────────────

pub async fn mcp_setup_get(
    config: &Config,
    qualified_name: String,
) -> Result<RpcOutcome<Value>, String> {
    let qualified_name = require(&qualified_name, "qualified_name")?;

    let (detail, required_env_keys) = resolve(config)?
        .dynamic()
        .registry_get(&qualified_name)
        .await
        .map_err(|error| error.to_string())?;

    Ok(RpcOutcome::new(
        json!({ "server": detail_payload(&detail, &required_env_keys)? }),
        vec![format!("setup_get ok qualified_name={qualified_name}")],
    ))
}

// ── request_secret ───────────────────────────────────────────────────────────

/// Mints a handle, asks the user for the value, and waits for it.
///
/// The wait is here rather than in `tinymcp` because the prompt is: this layer
/// publishes the event a user interface renders, so it is the layer that knows
/// when an answer can arrive.
pub async fn mcp_setup_request_secret(
    config: &Config,
    key_name: String,
    prompt: String,
) -> Result<RpcOutcome<Value>, String> {
    let key_name = require(&key_name, "key_name")?;
    let prompt = require(&prompt, "prompt")?;

    let service = resolve(config)?;
    let vault = service.dynamic().vault();
    let (handle, receiver) = vault.request(&key_name).await;

    BUS.publish(DomainEvent::McpSetupSecretRequested {
        ref_id: handle.as_str().to_string(),
        key_name: key_name.clone(),
        prompt,
    });
    tracing::info!(
        handle = handle.as_str(),
        key_name,
        "[mcp-setup] awaiting a secret from the user"
    );

    vault
        .await_fulfillment(&handle, receiver)
        .await
        .map_err(|error| error.to_string())?;

    tracing::info!(handle = handle.as_str(), "[mcp-setup] the secret arrived");

    Ok(RpcOutcome::new(
        json!({ "ref": handle.as_str(), "key_name": key_name }),
        vec![format!("collected secret for key={key_name}")],
    ))
}

// ── submit_secret ────────────────────────────────────────────────────────────

pub async fn mcp_setup_submit_secret(
    config: &Config,
    ref_id: String,
    value: String,
) -> Result<RpcOutcome<Value>, String> {
    let handle = SecretRef::parse(&ref_id).ok_or_else(|| format!("invalid ref_id `{ref_id}`"))?;

    let accepted = resolve(config)?
        .dynamic()
        .vault()
        .submit(&handle, value)
        .await;

    if !accepted {
        return Err(format!(
            "ref {} unknown or already submitted",
            handle.as_str()
        ));
    }

    Ok(RpcOutcome::new(
        json!({ "ref": handle.as_str(), "fulfilled": true }),
        vec![format!("submitted secret for ref={}", handle.as_str())],
    ))
}

// ── test_connection ──────────────────────────────────────────────────────────

/// Dials a server with the collected credentials without installing it.
///
/// A dial that fails is reported as `ok: false` with the reason rather than as
/// an error: the operation asked for — finding out whether it works — succeeded,
/// and the agent needs the reason to tell the user what to fix.
pub async fn mcp_setup_test_connection(
    config: &Config,
    qualified_name: String,
    env_refs: HashMap<String, String>,
) -> Result<RpcOutcome<Value>, String> {
    let qualified_name = require(&qualified_name, "qualified_name")?;
    let handles = parse_handles(env_refs)?;

    match resolve(config)?
        .dynamic()
        .setup_test_connection(&qualified_name, &handles)
        .await
    {
        Ok(tools) => {
            let tools = super::tools_safe_for_agent(&qualified_name, tools);
            let count = tools.len();
            Ok(RpcOutcome::new(
                json!({ "ok": true, "tools": tools }),
                vec![format!(
                    "test_connection ok for {qualified_name}: {count} tools"
                )],
            ))
        }
        Err(error) => Ok(RpcOutcome::new(
            json!({ "ok": false, "error": error.to_string() }),
            vec![format!(
                "test_connection failed for {qualified_name}: {error}"
            )],
        )),
    }
}

// ── install_and_connect ──────────────────────────────────────────────────────

/// The schema's `status` discriminator, and the `error` that accompanies a
/// non-connected result.
///
/// Split out as a pure function so both arms are testable without standing up a
/// registry and a reachable MCP server.
///
/// **Why the verdict comes from `ConnStatus` and not from the call's own
/// return.** `McpRegistry::setup_install_and_connect` turns a failed connect
/// into `Ok(ConnectOutcome { server_id, tools: vec![] })` — it logs the reason
/// and drops it. `ConnectOutcome` carries only `server_id` and `tools`, so a
/// failed connect is byte-identical to a server that connected and advertises
/// no tools, which is a legitimate state (a server exposing only resources or
/// prompts). Counting tools therefore cannot answer the question. The registry's
/// own `ConnStatus` can: it carries the live [`ServerStatus`] plus `last_error`.
///
/// A status we cannot read back is reported as `installed_disconnected` rather
/// than assumed connected: the install is confirmed either way, and claiming a
/// connection we did not observe is the failure mode this change exists to
/// remove (#6110).
///
/// The three inputs are deliberately distinct, because they are three different
/// facts and a caller acts on them differently:
///
/// - `Ok(Some(entry))` — the registry answered and knows this server.
/// - `Ok(None)` — the registry answered but has no row for it.
/// - `Err(reason)` — the status query itself failed. The reason is carried into
///   the message rather than being left in a log line the user never sees:
///   "the registry could not be asked" is not the same fact as "the server is
///   not connected", and a transient store error must not read as the latter.
fn classify_install_connect(
    status: Result<Option<&ConnStatus>, &str>,
) -> (&'static str, Option<String>) {
    match status {
        Ok(Some(entry)) if matches!(entry.status, ServerStatus::Connected) => ("connected", None),
        // `last_error` is the connect failure the registry recorded, not a
        // description of some later state: `connections::connect` builds a
        // `ConnectFailure` from the original error and stores it, and
        // `classify` surfaces it here. The synthetic arm below is reached only
        // when a non-connected server has no recorded failure at all (a
        // `Disabled` install, say), where naming the state is the whole answer.
        Ok(Some(entry)) => (
            "installed_disconnected",
            Some(entry.last_error.clone().unwrap_or_else(|| {
                format!(
                    "installed, but the server is `{}` rather than connected",
                    entry.status.as_str()
                )
            })),
        ),
        Ok(None) => (
            "installed_disconnected",
            Some("installed, but the registry reported no status row for this server".to_string()),
        ),
        Err(reason) => (
            "installed_disconnected",
            Some(format!(
                "installed, but the server's connection state could not be read back: {reason}"
            )),
        ),
    }
}

/// Builds the reply for `install_and_connect`, honouring the two conditional
/// fields the controller schema declares: `tools` iff `status == connected`,
/// `error` iff it is not. Pure, so the shape is testable without a registry.
fn install_and_connect_payload(
    server_id: &str,
    qualified_name: &str,
    status: &str,
    error: Option<String>,
    tools: Vec<tinymcp_bus::McpTool>,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("server_id".to_string(), json!(server_id));
    payload.insert("qualified_name".to_string(), json!(qualified_name));
    payload.insert("status".to_string(), json!(status));
    if status == "connected" {
        payload.insert("tools".to_string(), json!(tools));
    }
    if let Some(error) = error {
        payload.insert("error".to_string(), json!(error));
    }
    Value::Object(payload)
}

pub async fn mcp_setup_install_and_connect(
    config: &Config,
    qualified_name: String,
    env_refs: HashMap<String, String>,
) -> Result<RpcOutcome<Value>, String> {
    let qualified_name = require(&qualified_name, "qualified_name")?;
    let handles = parse_handles(env_refs)?;

    let host = resolve(config)?;
    // Still `?`: this arm is reached only when the *install* failed, and there
    // is no server to report a status for. A failed *connect* does not come
    // through here — see `classify_install_connect`.
    let outcome = host
        .dynamic()
        .setup_install_and_connect(&qualified_name, &handles, None)
        .await
        .map_err(|error| error.to_string())?;

    let server_id = outcome.server_id.clone();
    let connection = match host.dynamic().status().await {
        Ok(all) => Ok(all.into_iter().find(|entry| entry.server_id == server_id)),
        Err(error) => {
            log::warn!(
                "[mcp_setup] install_and_connect could not read back status for \
                 server_id={server_id}: {error}"
            );
            Err(error.to_string())
        }
    };
    let (status, error) = classify_install_connect(
        connection
            .as_ref()
            .map(Option::as_ref)
            .map_err(String::as_str),
    );
    let connected = status == "connected";

    let tools = super::tools_safe_for_agent(&server_id, outcome.tools);
    let tool_count = u32::try_from(tools.len()).unwrap_or(u32::MAX);

    BUS.publish(DomainEvent::McpServerInstalled {
        server_id: server_id.clone(),
        qualified_name: qualified_name.clone(),
    });
    // Only when the server is actually connected. This used to fire
    // unconditionally, so an install whose connect failed announced
    // `McpServerConnected { tool_count: 0 }` to every subscriber — the same
    // false claim the `status` field now avoids making to the caller (#6110).
    // A connected server with zero tools still publishes: `connected` is read
    // from `ServerStatus`, never from the tool count.
    if connected {
        BUS.publish(DomainEvent::McpServerConnected {
            server_id: server_id.clone(),
            tool_count,
        });
    }

    let log = if connected {
        format!("installed and connected server_id={server_id} tools={tool_count}")
    } else {
        format!(
            "installed server_id={server_id}, but connecting did not succeed: {}",
            error.as_deref().unwrap_or("no reason reported")
        )
    };
    Ok(RpcOutcome::new(
        install_and_connect_payload(&server_id, &qualified_name, status, error, tools),
        vec![log],
    ))
}

#[cfg(test)]
#[path = "setup_ops_tests.rs"]
mod tests;
