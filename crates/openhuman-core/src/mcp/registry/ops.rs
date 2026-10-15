//! RPC handler implementations for the MCP clients domain.
//!
//! Every function here maps one-to-one with a `schemas.rs` handler and keeps
//! the signature and the JSON shape it had before the client moved out to
//! `tinymcp` — the frontend calls these methods and nothing about its contract
//! changed. Registry, connection, and tool calls delegate to the configured
//! TinyMCP module object; host configuration and event projection remain here.
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
//! **The `mcp.json` document.** Installing a server is no longer a catalog
//! action: the user declares servers in one `mcp.json` document and
//! [`super::config_ops`] reconciles the store against it (`config_get` /
//! `config_set`). There is no install-from-catalog RPC: the app declares a
//! hosted server from the catalog into `mcp.json` itself.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{json, Value};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::Outcome;
use crate::mcp::host;

use tinymcp_bus::{
    explain_not_connected, not_connected_message, require_identifier as require,
    NotConnectedExplanation, NotConnectedRemedy,
};

/// Renders a value into the `Outcome` payload the frontend expects.
fn encode<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("serialization error: {error}"))
}

async fn module_call<R: serde::de::DeserializeOwned>(
    config: &Config,
    member: &str,
    args: impl serde::Serialize,
) -> Result<R, String> {
    super::super::module_client::McpConfiguredClient::for_config(config)
        .map_err(|error| error.to_string())?
        .call(&host::client_config(config), member, args)
        .await
        .map_err(|error| error.to_string())
}

async fn module_call_confidential<R: serde::de::DeserializeOwned>(
    config: &Config,
    member: &str,
    args: impl serde::Serialize,
) -> Result<R, String> {
    super::super::module_client::McpConfiguredClient::for_config(config)
        .map_err(|error| error.to_string())?
        .call_confidential(&host::client_config(config), member, args)
        .await
        .map_err(|error| error.to_string())
}

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
) -> Result<Outcome<Value>, String> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(20);

    let found: tinymcp_bus::RegistrySearchPage = module_call(
        config,
        tinymcp_bus::names::methods::REGISTRY_SEARCH_CURATED,
        (
            query,
            page,
            page_size,
            tinymcp_bus::SearchCuration {
                tag_official: true,
                official_first: true,
            },
        ),
    )
    .await?;

    let count = found.servers.len();
    Ok(Outcome::new(
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
) -> Result<Outcome<Value>, String> {
    let qualified_name = require(&qualified_name, "qualified_name")?;

    let detail: tinymcp_bus::ServerDetail = module_call(
        config,
        tinymcp_bus::names::methods::REGISTRY_GET,
        (qualified_name.clone(),),
    )
    .await?;

    // The registry tab shows what a server would ask for, and fetching the two
    // separately would be two catalog round trips for one screen.
    let server = detail.server_json()?;

    Ok(Outcome::new(
        json!({ "server": server }),
        vec![format!(
            "registry_get ok: {qualified_name} env_keys={}",
            detail.required_env_keys.len()
        )],
    ))
}

// ── installed_list ───────────────────────────────────────────────────────────

pub async fn mcp_clients_installed_list(config: &Config) -> Result<Outcome<Value>, String> {
    let installed: Vec<tinymcp_bus::InstalledServer> =
        module_call(config, tinymcp_bus::names::methods::INSTALLED_LIST, ()).await?;

    let count = installed.len();
    Ok(Outcome::new(
        json!({ "installed": installed }),
        vec![format!("installed_list returned {count} servers")],
    ))
}

// ── uninstall ────────────────────────────────────────────────────────────────

pub async fn mcp_clients_uninstall(
    config: &Config,
    server_id: String,
) -> Result<Outcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let removed: bool = module_call(
        config,
        tinymcp_bus::names::methods::UNINSTALL,
        (server_id.clone(),),
    )
    .await?;

    Ok(Outcome::new(
        json!({ "server_id": server_id, "removed": removed }),
        vec![format!("uninstalled server_id={server_id}")],
    ))
}

// ── auth detection and browser OAuth ─────────────────────────────────────────

pub async fn mcp_clients_detect_auth(
    config: &Config,
    server_id: String,
) -> Result<Outcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let detection: tinymcp_bus::AuthDetection = module_call(
        config,
        tinymcp_bus::names::methods::DETECT_AUTH,
        (server_id.clone(),),
    )
    .await?;

    let kind = detection.kind.as_str();
    let value = encode(&detection)?;

    Ok(Outcome::new(
        value,
        vec![format!("detect_auth {server_id} -> {kind}")],
    ))
}

pub async fn mcp_clients_oauth_begin(
    config: &Config,
    server_id: String,
) -> Result<Outcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let authorize_url: String = module_call(
        config,
        tinymcp_bus::names::methods::OAUTH_BEGIN,
        (server_id.clone(), host::oauth_redirect_uri()),
    )
    .await?;

    Ok(Outcome::new(
        json!({ "authorize_url": authorize_url }),
        vec![format!("oauth_begin {server_id}")],
    ))
}

// ── connect ──────────────────────────────────────────────────────────────────

pub async fn mcp_clients_connect(
    config: &Config,
    server_id: String,
) -> Result<Outcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let outcome: tinymcp_bus::ConnectOutcome = module_call(
        config,
        tinymcp_bus::names::methods::CONNECT,
        (server_id.clone(),),
    )
    .await?;

    let tools = super::tools_safe_for_agent_scanned(&server_id, outcome.tools).await;
    let tool_count = u32::try_from(tools.len()).unwrap_or(u32::MAX);

    BUS.publish(DomainEvent::McpServerConnected {
        server_id: server_id.clone(),
        tool_count,
    });

    Ok(Outcome::new(
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
) -> Result<Outcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let _: () = module_call(
        config,
        tinymcp_bus::names::methods::SET_ENABLED,
        (server_id.clone(), enabled),
    )
    .await?;

    if !enabled {
        BUS.publish(DomainEvent::McpServerDisconnected {
            server_id: server_id.clone(),
            reason: Some("disabled".to_string()),
        });
    }

    Ok(Outcome::new(
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
) -> Result<Outcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;

    let _: bool = module_call(
        config,
        tinymcp_bus::names::methods::DISCONNECT,
        (server_id.clone(),),
    )
    .await?;

    BUS.publish(DomainEvent::McpServerDisconnected {
        server_id: server_id.clone(),
        reason: None,
    });

    Ok(Outcome::new(
        json!({ "server_id": server_id, "status": "disconnected" }),
        vec![format!("disconnected server_id={server_id}")],
    ))
}

// ── update_env ───────────────────────────────────────────────────────────────

pub async fn mcp_clients_update_env(
    config: &Config,
    server_id: String,
    env: HashMap<String, String>,
) -> Result<Outcome<Value>, String> {
    use tinymcp_bus::UpdateEnvStatus;

    let server_id = require(&server_id, "server_id")?;

    let outcome: tinymcp_bus::UpdateEnvOutcome = module_call_confidential(
        config,
        tinymcp_bus::names::methods::UPDATE_ENV,
        (
            server_id.clone(),
            env.into_iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
        ),
    )
    .await?;

    match outcome.status {
        UpdateEnvStatus::Connected => {
            let tools = super::tools_safe_for_agent_scanned(&server_id, outcome.tools).await;
            let tool_count = u32::try_from(tools.len()).unwrap_or(u32::MAX);

            BUS.publish(DomainEvent::McpServerConnected {
                server_id: server_id.clone(),
                tool_count,
            });

            Ok(Outcome::new(
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
        UpdateEnvStatus::Disabled => Ok(Outcome::new(
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
            Ok(Outcome::new(
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
            Ok(Outcome::new(
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

pub async fn mcp_clients_registry_settings_get(config: &Config) -> Result<Outcome<Value>, String> {
    let settings: tinymcp_bus::RegistrySettings = module_call(
        config,
        tinymcp_bus::names::methods::REGISTRY_SETTINGS_GET,
        (),
    )
    .await?;

    Ok(Outcome::new(
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
) -> Result<Outcome<Value>, String> {
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

    let settings: tinymcp_bus::RegistrySettings = module_call_confidential(
        config,
        tinymcp_bus::names::methods::REGISTRY_SETTINGS_SET,
        (smithery_api_key, mcp_official_base, mcp_official_token),
    )
    .await?;

    Ok(Outcome::new(
        encode(&settings)?,
        vec!["registry_settings_set saved".to_string()],
    ))
}

// ── status ───────────────────────────────────────────────────────────────────

pub async fn mcp_clients_status(config: &Config) -> Result<Outcome<Value>, String> {
    let statuses: Vec<tinymcp_bus::ConnStatus> =
        module_call(config, tinymcp_bus::names::methods::STATUS, ()).await?;

    let count = statuses.len();
    Ok(Outcome::new(
        json!({ "servers": statuses }),
        vec![format!("status returned {count} servers")],
    ))
}

// ── list_tools ───────────────────────────────────────────────────────────────

/// Who is listing tools, which decides the remedy a refusal names.
///
/// The RPC and the agent tool share one implementation but not one surface: a
/// settings-UI caller can call `mcp_clients_connect`, a model can only call the
/// tools on its belt. Naming a method the caller cannot reach is an instruction
/// it cannot follow (#6313).
///
/// Not every belt that lists tools can connect: the read-only planner carries
/// `mcp_registry_list_tools` and `mcp_registry_status` but not
/// `mcp_registry_connect`. So an agent refusal names no connect tool, only the
/// status tool every such belt has; `refusals_name_only_tools_on_every_listing_belt`
/// pins that against the built-in agent definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// The `openhuman.mcp_clients_*` RPC surface (settings UI, CLI).
    Rpc,
    /// The model, through the `mcp_registry_*` tools on its belt.
    Agent,
}

impl Caller {
    /// The tools this caller can reach, which decides what a refusal names.
    fn remedy(self) -> NotConnectedRemedy<'static> {
        match self {
            Self::Rpc => NotConnectedRemedy {
                connect: Some("mcp_clients_connect"),
                status: "mcp_clients_status",
            },
            Self::Agent => NotConnectedRemedy {
                connect: None,
                status: "mcp_registry_status",
            },
        }
    }
}

pub async fn mcp_clients_list_tools(
    config: &Config,
    server_id: String,
    caller: Caller,
) -> Result<Outcome<Value>, String> {
    let mut server_id = require(&server_id, "server_id")?;
    let tools: Vec<tinymcp_bus::McpTool> = match module_call(
        config,
        tinymcp_bus::names::methods::LIST_TOOLS,
        (server_id.clone(),),
    )
    .await
    {
        Ok(tools) => tools,
        Err(error) => {
            let installs: Vec<tinymcp_bus::ConnStatus> =
                module_call(config, tinymcp_bus::names::methods::STATUS, ()).await?;
            if installs.iter().any(|install| {
                install.server_id == server_id
                    && install.status == tinymcp_bus::ServerStatus::Connected
            }) {
                return Err(error);
            }
            match explain_not_connected(&server_id, &installs, caller.remedy()) {
                NotConnectedExplanation::Resolved(resolved) => {
                    server_id = resolved;
                    module_call(
                        config,
                        tinymcp_bus::names::methods::LIST_TOOLS,
                        (server_id.clone(),),
                    )
                    .await
                    .map_err(|_| {
                        installs
                            .iter()
                            .find(|install| install.server_id == server_id)
                            .map_or(error.clone(), |install| {
                                not_connected_message(install, caller.remedy())
                            })
                    })?
                }
                NotConnectedExplanation::Refused(message) => return Err(message),
                _ => return Err(error),
            }
        }
    };

    let tools = super::tools_safe_for_agent_scanned(&server_id, tools).await;
    let count = tools.len();

    Ok(Outcome::new(
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
) -> Result<Outcome<Value>, String> {
    let server_id = require(&server_id, "server_id")?;
    let tool_name = require(&tool_name, "tool_name")?;

    let start = Instant::now();
    let result: Result<tinymcp_bus::ToolCallOutcome, String> = module_call(
        config,
        tinymcp_bus::names::methods::TOOL_CALL,
        (server_id.clone(), tool_name.clone(), arguments),
    )
    .await;
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    BUS.publish(DomainEvent::McpClientToolExecuted {
        server_id: server_id.clone(),
        tool_name: tool_name.clone(),
        success: result.is_ok(),
        elapsed_ms,
    });

    match result {
        Ok(outcome) => Ok(Outcome::new(
            json!({ "result": outcome.result, "is_error": outcome.is_error }),
            vec![format!(
                "tool_call ok server_id={server_id} tool={tool_name} elapsed_ms={elapsed_ms}"
            )],
        )),
        Err(error) => Ok(Outcome::new(
            json!({ "result": error, "is_error": true }),
            vec![format!(
                "tool_call error server_id={server_id} tool={tool_name}: {error}"
            )],
        )),
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
