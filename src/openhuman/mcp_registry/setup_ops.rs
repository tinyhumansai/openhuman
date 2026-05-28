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
use crate::openhuman::mcp_client::{McpHttpClient, McpStdioClient};
use crate::rpc::RpcOutcome;

use super::ops::resolve_command;
use super::setup::{self, SecretRef};
use super::types::{CommandKind, InstalledServer, SmitheryConnection, Transport};
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
    let picked = pick_connection(&detail.connections).ok_or_else(|| {
        format!("server `{q}` exposes neither stdio nor http_remote connections; nothing to test")
    })?;

    let identity = config.mcp_client.client_identity.clone();

    // Scratch session — initialise + list_tools, then close. Nothing
    // persisted. Errors bubble up so the agent can show them to the user.
    let (init_ok, tools) = match picked.transport_kind() {
        "stdio" => {
            let (_kind, command, args) = resolve_command(q, Some(picked));
            let cwd: Option<PathBuf> = None;
            let client = McpStdioClient::new(command, args, env, cwd, identity);
            if let Err(err) = client.initialize().await {
                return Ok(RpcOutcome::new(
                    json!({ "ok": false, "error": err.to_string() }),
                    vec![format!("test_connection failed for {q}: {err}")],
                ));
            }
            match client.list_tools().await {
                Ok(t) => {
                    let _ = client.close_session().await;
                    (true, t)
                }
                Err(err) => {
                    let _ = client.close_session().await;
                    return Ok(RpcOutcome::new(
                        json!({ "ok": false, "error": err.to_string() }),
                        vec![format!("test_connection list_tools failed for {q}: {err}")],
                    ));
                }
            }
        }
        // HTTP-remote path: dial the published deployment_url over
        // Streamable HTTP. No subprocess, no env injection needed at
        // dial time (env vars for HTTP-remote installs are typically
        // OAuth tokens that the McpHttpClient picks up from its own
        // auth config — out of scope for this scratch test).
        "http_remote" => {
            let endpoint = picked.deployment_url.clone().unwrap_or_default();
            if endpoint.is_empty() {
                return Ok(RpcOutcome::new(
                    json!({ "ok": false, "error": "deployment_url is empty for http_remote connection" }),
                    vec![format!(
                        "test_connection failed for {q}: empty deployment_url"
                    )],
                ));
            }
            let client = McpHttpClient::new(endpoint.clone(), 30);
            if let Err(err) = client.initialize().await {
                return Ok(RpcOutcome::new(
                    json!({ "ok": false, "error": err.to_string() }),
                    vec![format!("test_connection (http) failed for {q}: {err}")],
                ));
            }
            match client.list_tools().await {
                Ok(t) => {
                    let _ = client.close_session().await;
                    (true, t)
                }
                Err(err) => {
                    let _ = client.close_session().await;
                    return Ok(RpcOutcome::new(
                        json!({ "ok": false, "error": err.to_string() }),
                        vec![format!(
                            "test_connection (http) list_tools failed for {q}: {err}"
                        )],
                    ));
                }
            }
        }
        other => {
            return Ok(RpcOutcome::new(
                json!({ "ok": false, "error": format!("unsupported transport `{other}`") }),
                vec![format!(
                    "test_connection failed for {q}: unsupported transport `{other}`"
                )],
            ));
        }
    };

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    Ok(RpcOutcome::new(
        json!({ "ok": init_ok, "tools": tools, "transport": picked.transport_kind() }),
        vec![format!(
            "test_connection ok for {q} via {}: {} tools ({:?})",
            picked.transport_kind(),
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
    let picked = pick_connection(&detail.connections).ok_or_else(|| {
        format!(
            "server `{q}` exposes neither stdio nor http_remote connections; nothing to install"
        )
    })?;

    // Branch on the picked transport. Stdio installs still populate
    // command/args (current behavior). HTTP-remote installs leave them
    // empty and stash the deployment URL in `transport`.
    let (transport, command_kind, command, args) = match picked.transport_kind() {
        "http_remote" => {
            let url = picked.deployment_url.clone().unwrap_or_default();
            if url.is_empty() {
                return Err(format!(
                    "server `{q}` http_remote connection has empty deployment_url"
                ));
            }
            (
                Transport::HttpRemote { url },
                CommandKind::Node, // unused for HTTP, but a sensible default
                String::new(),
                Vec::new(),
            )
        }
        _ => {
            let (kind, command, args) = resolve_command(q, Some(picked));
            (Transport::Stdio, kind, command, args)
        }
    };

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
        transport,
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

/// Choose the best [`SmitheryConnection`] from a registry detail response.
///
/// Preference order:
/// 1. **Published `stdio`** — no behaviour regression for any server that
///    used to install before HTTP-remote support landed.
/// 2. **Any `stdio`** (even unpublished) — also pre-existing fallback.
/// 3. **Published `http_remote`** — the new path. Smithery serves ~99% of
///    their listings as HTTP-remote.
/// 4. **Any `http_remote`** — last-resort.
/// 5. `None` — nothing dialable.
///
/// Stdio is preferred because it's privacy-strict (everything runs locally)
/// and because most of the existing OpenHuman ecosystem assumes stdio. HTTP
/// is the fallback that finally lets a Smithery-only server install at all.
pub(super) fn pick_connection(connections: &[SmitheryConnection]) -> Option<&SmitheryConnection> {
    // Treat the canonical wire names ("stdio", "http") AND the persisted
    // dispatch kinds ("http_remote") as equivalent — registry payloads
    // historically use "http" while our `Transport` discriminator uses
    // "http_remote". `transport_kind` normalises that mapping.
    let stdio_pub = connections
        .iter()
        .find(|c| c.transport_kind() == "stdio" && c.published);
    if stdio_pub.is_some() {
        return stdio_pub;
    }
    let stdio_any = connections.iter().find(|c| c.transport_kind() == "stdio");
    if stdio_any.is_some() {
        return stdio_any;
    }
    let http_pub = connections
        .iter()
        .find(|c| c.transport_kind() == "http_remote" && c.published);
    if http_pub.is_some() {
        return http_pub;
    }
    connections
        .iter()
        .find(|c| c.transport_kind() == "http_remote")
}

/// Normalise a [`SmitheryConnection::r#type`] string into the same vocabulary
/// the persisted [`Transport`] enum uses. The registry side uses `"http"`
/// in its DTOs; we route those into the `"http_remote"` install path.
trait ConnectionKind {
    fn transport_kind(&self) -> &str;
}

impl ConnectionKind for SmitheryConnection {
    fn transport_kind(&self) -> &str {
        match self.r#type.as_str() {
            "stdio" => "stdio",
            "http" | "http_remote" | "sse" => "http_remote",
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(kind: &str, published: bool, url: Option<&str>) -> SmitheryConnection {
        SmitheryConnection {
            r#type: kind.to_string(),
            deployment_url: url.map(String::from),
            config_schema: None,
            example_config: None,
            published,
            extra: std::collections::HashMap::new(),
        }
    }

    /// Stdio wins when both transports are offered, even when stdio is
    /// unpublished and http is published. This pins the "no regression
    /// for existing stdio installs" promise.
    #[test]
    fn pick_connection_prefers_stdio_over_http() {
        let conns = vec![
            conn("http", true, Some("https://x.io/mcp")),
            conn("stdio", false, None),
        ];
        let picked = pick_connection(&conns).expect("stdio should be picked");
        assert_eq!(picked.r#type, "stdio");
    }

    /// Published stdio beats unpublished stdio.
    #[test]
    fn pick_connection_prefers_published_stdio_first() {
        let conns = vec![conn("stdio", false, None), conn("stdio", true, None)];
        let picked = pick_connection(&conns).expect("published stdio should win");
        assert!(picked.published);
    }

    /// When the server is HTTP-remote-only (the Smithery-typical case),
    /// the picker returns the HTTP-remote connection instead of `None` —
    /// this is the core gap the PR closes.
    #[test]
    fn pick_connection_falls_back_to_http_remote_when_no_stdio() {
        let conns = vec![conn("http", true, Some("https://x.io/mcp"))];
        let picked = pick_connection(&conns).expect("http_remote fallback");
        assert_eq!(picked.transport_kind(), "http_remote");
        assert_eq!(picked.deployment_url.as_deref(), Some("https://x.io/mcp"));
    }

    /// Smithery DTOs use `"http"`, our `Transport` discriminator uses
    /// `"http_remote"`. Normalisation pins both as the same install path.
    #[test]
    fn connection_kind_normalises_http_variants() {
        assert_eq!(conn("http", true, None).transport_kind(), "http_remote");
        assert_eq!(
            conn("http_remote", true, None).transport_kind(),
            "http_remote"
        );
        assert_eq!(conn("sse", true, None).transport_kind(), "http_remote");
        assert_eq!(conn("stdio", true, None).transport_kind(), "stdio");
        // Unknown kinds fall through untouched so the picker can ignore them.
        assert_eq!(conn("ws", true, None).transport_kind(), "ws");
    }

    /// No dialable connection → picker returns None so callers can return
    /// a clean error instead of dialing garbage.
    #[test]
    fn pick_connection_returns_none_for_only_unknown_kinds() {
        let conns = vec![conn("websocket-future", true, None)];
        assert!(pick_connection(&conns).is_none());
    }
}
