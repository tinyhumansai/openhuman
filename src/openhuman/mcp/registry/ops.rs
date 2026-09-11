//! RPC handler implementations for the MCP clients domain.
//!
//! Every function here maps one-to-one with a `schemas.rs` handler and keeps
//! the signature and the JSON shape it had before the client moved out to
//! `tinymcp` — the frontend calls these methods and nothing about its contract
//! changed. What changed is underneath: each one now delegates to the service
//! [`super::super::host`] holds.
//!
//! # What stayed here on purpose
//!
//! **Events.** `tinymcp` reports what happened in its return values and
//! publishes nothing. `DomainEvent` is this application's vocabulary, so the
//! publishing happens at this layer, where that vocabulary is known.
//!
//! **The scan over remote tool definitions.** Prompt-injection detection is
//! host policy: the detector, its rules, and what a hit means belong to this
//! application's threat model.
//!
//! **The configuration assistant's agent turn.** `tinymcp` gathers the catalog
//! detail and the credential names a model would need and stops. Running the
//! turn needs the agent, the tool surface, and the approval gate, all of which
//! are here.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{json, Value};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;
use crate::openhuman::mcp::host;
use crate::rpc::RpcOutcome;

use super::helpers::{encode, inject_required_env_keys, require, resolve};
use super::types::ChatTurn;

// ── registry_search ──────────────────────────────────────────────────────────

/// `transport` is accepted and ignored: the catalog no longer filters by it,
/// because the picker chooses a connection at install time from what the server
/// actually offers. The parameter stays so the frontend's call does not have to
/// change in the same release.
pub async fn mcp_clients_registry_search(
    config: &Config,
    query: Option<String>,
    _transport: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<RpcOutcome<Value>, String> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(20);

    let mut found = resolve(config)?
        .dynamic()
        .registry_search(query.as_deref(), page, page_size)
        .await
        .map_err(|error| error.to_string())?;

    // Badging and the strict filter are presentation choices, applied here so a
    // caller assembling its own view does not have to undo them.
    tinymcp::registry::curation::tag_official(&mut found.servers);
    tinymcp::registry::curation::float_official_first(&mut found.servers);

    let count = found.servers.len();
    Ok(RpcOutcome::new(
        json!({
            "servers": found.servers,
            "page": found.page,
            "total_pages": found.total_pages,
        }),
        vec![format!("registry_search returned {count} servers")],
    ))
}

// ── registry_get ─────────────────────────────────────────────────────────────

pub async fn mcp_clients_registry_get(
    config: &Config,
    qualified_name: String,
) -> Result<RpcOutcome<Value>, String> {
    let qualified_name = require(&qualified_name, "qualified_name")?;

    let (detail, required_env_keys) = resolve(config)?
        .dynamic()
        .registry_get(&qualified_name)
        .await
        .map_err(|error| error.to_string())?;

    // The install dialog needs both, and fetching them separately would be two
    // catalog round trips for one screen.
    let mut server = encode(&detail)?;
    inject_required_env_keys(&mut server, &required_env_keys);

    Ok(RpcOutcome::new(
        json!({ "server": server }),
        vec![format!(
            "registry_get ok: {qualified_name} env_keys={}",
            required_env_keys.len()
        )],
    ))
}

// ── installed_list ───────────────────────────────────────────────────────────

pub async fn mcp_clients_installed_list(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let installed = resolve(config)?
        .dynamic()
        .installed_list()
        .map_err(|error| error.to_string())?;

    let count = installed.len();
    Ok(RpcOutcome::new(
        json!({ "installed": installed }),
        vec![format!("installed_list returned {count} servers")],
    ))
}

// ── install ──────────────────────────────────────────────────────────────────

pub async fn mcp_clients_install(
    config: &Config,
    qualified_name: String,
    env: HashMap<String, String>,
    config_value: Option<Value>,
) -> Result<RpcOutcome<Value>, String> {
    let qualified_name = require(&qualified_name, "qualified_name")?;

    let outcome = resolve(config)?
        .dynamic()
        .install(&qualified_name, env.into_iter().collect(), config_value)
        .await
        .map_err(|error| error.to_string())?;

    if !outcome.already_installed {
        BUS.publish(DomainEvent::McpServerInstalled {
            server_id: outcome.server.server_id.clone(),
            qualified_name: outcome.server.qualified_name.clone(),
        });
    }

    let note = if outcome.already_installed {
        format!("already installed qualified_name={qualified_name}")
    } else {
        format!("installed server_id={}", outcome.server.server_id)
    };

    Ok(RpcOutcome::new(
        json!({
            "server": outcome.server,
            "already_installed": outcome.already_installed,
        }),
        vec![note],
    ))
}

// ── uninstall ────────────────────────────────────────────────────────────────

pub async fn mcp_clients_uninstall(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let removed = resolve(config)?
        .dynamic()
        .uninstall(&server_id)
        .await
        .map_err(|error| error.to_string())?;

    Ok(RpcOutcome::new(
        json!({ "server_id": server_id, "removed": removed }),
        vec![format!("uninstalled server_id={server_id}")],
    ))
}

// ── auth detection and browser OAuth ─────────────────────────────────────────

pub async fn mcp_clients_detect_auth(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let detection = resolve(config)?
        .dynamic()
        .detect_auth(&server_id)
        .await
        .map_err(|error| error.to_string())?;

    let kind = detection.kind.as_str();
    let value = encode(&detection)?;

    Ok(RpcOutcome::new(
        value,
        vec![format!("detect_auth {server_id} -> {kind}")],
    ))
}

pub async fn mcp_clients_oauth_begin(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let authorize_url = resolve(config)?
        .dynamic()
        .oauth_begin(&server_id, &host::oauth_redirect_uri())
        .await
        .map_err(|error| error.to_string())?;

    Ok(RpcOutcome::new(
        json!({ "authorize_url": authorize_url }),
        vec![format!("oauth_begin {server_id}")],
    ))
}

// ── connect ──────────────────────────────────────────────────────────────────

pub async fn mcp_clients_connect(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let outcome = resolve(config)?
        .dynamic()
        .connect(&server_id)
        .await
        .map_err(|error| error.to_string())?;

    let tools = super::tools_safe_for_agent(&server_id, outcome.tools);
    let tool_count = u32::try_from(tools.len()).unwrap_or(u32::MAX);

    BUS.publish(DomainEvent::McpServerConnected {
        server_id: server_id.clone(),
        tool_count,
    });

    Ok(RpcOutcome::new(
        json!({ "server_id": server_id, "status": "connected", "tools": tools }),
        vec![format!(
            "connected server_id={server_id} tools={tool_count}"
        )],
    ))
}

// ── set_enabled ──────────────────────────────────────────────────────────────

pub async fn mcp_clients_set_enabled(
    config: &Config,
    server_id: String,
    enabled: bool,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    resolve(config)?
        .dynamic()
        .set_enabled(&server_id, enabled)
        .await
        .map_err(|error| error.to_string())?;

    if !enabled {
        BUS.publish(DomainEvent::McpServerDisconnected {
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

// ── disconnect ───────────────────────────────────────────────────────────────

pub async fn mcp_clients_disconnect(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    resolve(config)?
        .dynamic()
        .disconnect(&server_id)
        .await
        .map_err(|error| error.to_string())?;

    BUS.publish(DomainEvent::McpServerDisconnected {
        server_id: server_id.clone(),
        reason: None,
    });

    Ok(RpcOutcome::new(
        json!({ "server_id": server_id, "status": "disconnected" }),
        vec![format!("disconnected server_id={server_id}")],
    ))
}

// ── update_env ───────────────────────────────────────────────────────────────

pub async fn mcp_clients_update_env(
    config: &Config,
    server_id: String,
    env: HashMap<String, String>,
) -> Result<RpcOutcome<Value>, String> {
    use tinymcp_bus::UpdateEnvStatus;

    let server_id = require(&server_id, "server_id")?;

    let outcome = resolve(config)?
        .dynamic()
        .update_env(&server_id, env.into_iter().collect())
        .await
        .map_err(|error| error.to_string())?;

    match outcome.status {
        UpdateEnvStatus::Connected => {
            let tools = super::tools_safe_for_agent(&server_id, outcome.tools);
            let tool_count = u32::try_from(tools.len()).unwrap_or(u32::MAX);

            BUS.publish(DomainEvent::McpServerConnected {
                server_id: server_id.clone(),
                tool_count,
            });

            Ok(RpcOutcome::new(
                json!({
                    "server_id": server_id,
                    "status": "connected",
                    "env_keys": outcome.env_keys,
                    "tools": tools,
                }),
                vec![format!(
                    "update_env reconnected server_id={server_id} tools={tool_count}"
                )],
            ))
        }
        UpdateEnvStatus::Disabled => Ok(RpcOutcome::new(
            json!({
                "server_id": server_id,
                "status": "disabled",
                "env_keys": outcome.env_keys,
            }),
            vec![format!(
                "update_env persisted env for server_id={server_id} but did not reconnect: server is disabled"
            )],
        )),
        UpdateEnvStatus::Unauthorized => {
            // The reason code, never the raw 401 message: that leaks the OAuth
            // metadata URL, and the frontend renders localized copy from the
            // code alone.
            let hint = outcome.auth_hint.map(|hint| hint.as_code()).unwrap_or_default();
            Ok(RpcOutcome::new(
                json!({
                    "server_id": server_id,
                    "status": "unauthorized",
                    "env_keys": outcome.env_keys,
                    "auth_hint": hint,
                }),
                vec![format!(
                    "update_env persisted env for server_id={server_id} but reconnect was unauthorized: {hint}"
                )],
            ))
        }
        _ => {
            let error = outcome.error.unwrap_or_default();
            Ok(RpcOutcome::new(
                json!({
                    "server_id": server_id,
                    "status": "disconnected",
                    "env_keys": outcome.env_keys,
                    "error": error,
                }),
                vec![format!(
                    "update_env persisted env for server_id={server_id} but reconnect failed: {error}"
                )],
            ))
        }
    }
}

// ── registry settings ────────────────────────────────────────────────────────

pub async fn mcp_clients_registry_settings_get(
    config: &Config,
) -> Result<RpcOutcome<Value>, String> {
    let settings = resolve(config)?.dynamic().registry_settings();

    Ok(RpcOutcome::new(
        encode(&settings)?,
        vec!["registry_settings_get".to_string()],
    ))
}

/// Persists the credentials and tells the running service about them.
///
/// Both halves are needed: the file is what survives a restart, and the service
/// is what the next search actually uses.
pub async fn mcp_clients_registry_settings_set(
    config: &mut Config,
    smithery_api_key: Option<String>,
    mcp_official_base: Option<String>,
    mcp_official_token: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    /// A blank update clears the field; an absent one leaves it.
    fn apply(field: &mut Option<String>, update: Option<String>) {
        if let Some(value) = update {
            let trimmed = value.trim();
            *field = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
    }

    let auth = &mut config.mcp_client.registry_auth;
    apply(&mut auth.smithery_api_key, smithery_api_key.clone());
    apply(&mut auth.mcp_official_base, mcp_official_base.clone());
    apply(&mut auth.mcp_official_token, mcp_official_token.clone());

    config.save().await.map_err(|error| error.to_string())?;

    let settings = resolve(config)?.dynamic().set_registry_settings(
        smithery_api_key,
        mcp_official_base,
        mcp_official_token,
    );

    Ok(RpcOutcome::new(
        encode(&settings)?,
        vec!["registry_settings_set saved".to_string()],
    ))
}

// ── status ───────────────────────────────────────────────────────────────────

pub async fn mcp_clients_status(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let statuses = resolve(config)?
        .dynamic()
        .status()
        .await
        .map_err(|error| error.to_string())?;

    let count = statuses.len();
    Ok(RpcOutcome::new(
        json!({ "servers": statuses }),
        vec![format!("status returned {count} servers")],
    ))
}

// ── list_tools ───────────────────────────────────────────────────────────────

pub async fn mcp_clients_list_tools(
    config: &Config,
    server_id: String,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    /// What a caller has to do about it either way.
    fn connect_first(server_id: &str) -> String {
        format!("server_id={server_id} is not connected; connect it first via mcp_clients_connect")
    }

    let tools = resolve(config)?
        .dynamic()
        .list_tools(&server_id)
        .await
        .map_err(|error| {
            tracing::debug!("[mcp-client] list_tools ({server_id}) failed: {error}");
            connect_first(&server_id)
        })?;

    let tools = super::tools_safe_for_agent(&server_id, tools);
    let count = tools.len();

    Ok(RpcOutcome::new(
        json!({ "server_id": server_id, "tools": tools }),
        vec![format!(
            "list_tools server_id={server_id} returned {count} tools"
        )],
    ))
}

// ── tool_call ────────────────────────────────────────────────────────────────

pub async fn mcp_clients_tool_call(
    config: &Config,
    server_id: String,
    tool_name: String,
    arguments: Value,
) -> Result<RpcOutcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;
    let tool_name = require(&tool_name, "tool_name")?;

    let start = Instant::now();
    let result = resolve(config)?
        .dynamic()
        .tool_call(&server_id, &tool_name, arguments)
        .await;
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    BUS.publish(DomainEvent::McpClientToolExecuted {
        server_id: server_id.clone(),
        tool_name: tool_name.clone(),
        success: result.is_ok(),
        elapsed_ms,
    });

    match result {
        Ok(outcome) => Ok(RpcOutcome::new(
            json!({ "result": outcome.result, "is_error": outcome.is_error }),
            vec![format!(
                "tool_call ok server_id={server_id} tool={tool_name} elapsed_ms={elapsed_ms}"
            )],
        )),
        Err(error) => Ok(RpcOutcome::new(
            json!({ "result": error.to_string(), "is_error": true }),
            vec![format!(
                "tool_call error server_id={server_id} tool={tool_name}: {error}"
            )],
        )),
    }
}

// ── config_assist ────────────────────────────────────────────────────────────

pub async fn mcp_clients_config_assist(
    config: &Config,
    qualified_name: String,
    user_message: String,
    history: Option<Vec<ChatTurn>>,
) -> Result<RpcOutcome<Value>, String> {
    let qualified_name = require(&qualified_name, "qualified_name")?;

    tracing::debug!(
        "[mcp-client] config_assist qualified_name={} message_len={}",
        qualified_name,
        user_message.len()
    );

    // The module gathers the catalog detail and the credential names; running
    // the turn below is this layer's, because it needs the agent, the tool
    // surface and the approval gate.
    let (detail, required_env_keys) = resolve(config)?
        .dynamic()
        .config_assist(&qualified_name)
        .await
        .map_err(|error| format!("Failed to fetch registry detail: {error}"))?;

    let system_prompt = build_config_assist_system_prompt(
        &detail.display_name,
        &qualified_name,
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

/// Invoke a lightweight inference call for config_assist.
/// Uses the existing `inference` domain to run a structured-output chat turn.
async fn invoke_config_assist_agent(
    config: &Config,
    // The legacy JSON-asking system prompt is intentionally unused: the agent
    // turn returns its text verbatim, so we want natural markdown, not a JSON
    // envelope. Server context comes through `user_message`.
    _system_prompt: &str,
    history: &[ChatTurn],
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
                    "Couldn't start the assistant: {e}. Make sure AI/inference is configured (Connections → API keys → LLM)."
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
                "I couldn't research that right now: {e}. Make sure AI/inference is configured (Connections → API keys → LLM)."
            ),
            "suggested_env": null
        })),
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
