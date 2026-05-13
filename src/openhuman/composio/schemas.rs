//! Controller schemas + registered handlers for the Composio domain.
//!
//! Exposes the domain over the shared registry at
//! `openhuman.composio_*`:
//!   - `composio.list_toolkits`       → `openhuman.composio_list_toolkits`
//!   - `composio.list_connections`    → `openhuman.composio_list_connections`
//!   - `composio.authorize`           → `openhuman.composio_authorize`
//!   - `composio.delete_connection`   → `openhuman.composio_delete_connection`
//!   - `composio.list_tools`          → `openhuman.composio_list_tools`
//!   - `composio.execute`             → `openhuman.composio_execute`
//!   - `composio.list_github_repos`   → `openhuman.composio_list_github_repos`
//!   - `composio.create_trigger`      → `openhuman.composio_create_trigger`
//!   - `composio.get_user_profile`    → `openhuman.composio_get_user_profile`
//!   - `composio.refresh_all_identities` → `openhuman.composio_refresh_all_identities`
//!   - `composio.sync`                → `openhuman.composio_sync`

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, serde::Deserialize)]
struct TriggerHistoryParams {
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
struct ListGithubReposParams {
    connection_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CreateTriggerParams {
    slug: String,
    connection_id: Option<String>,
    trigger_config: Option<Value>,
}

#[derive(Debug, serde::Deserialize)]
struct ListAvailableTriggersParams {
    toolkit: String,
    connection_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ListTriggersParams {
    toolkit: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct EnableTriggerParams {
    connection_id: String,
    slug: String,
    trigger_config: Option<Value>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_toolkits"),
        schemas("list_connections"),
        schemas("authorize"),
        schemas("delete_connection"),
        schemas("list_tools"),
        schemas("execute"),
        schemas("list_github_repos"),
        schemas("create_trigger"),
        schemas("get_user_profile"),
        schemas("refresh_all_identities"),
        schemas("sync"),
        schemas("list_trigger_history"),
        schemas("get_user_scopes"),
        schemas("set_user_scopes"),
        schemas("list_available_triggers"),
        schemas("list_triggers"),
        schemas("enable_trigger"),
        schemas("disable_trigger"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_toolkits"),
            handler: handle_list_toolkits,
        },
        RegisteredController {
            schema: schemas("list_connections"),
            handler: handle_list_connections,
        },
        RegisteredController {
            schema: schemas("authorize"),
            handler: handle_authorize,
        },
        RegisteredController {
            schema: schemas("delete_connection"),
            handler: handle_delete_connection,
        },
        RegisteredController {
            schema: schemas("list_tools"),
            handler: handle_list_tools,
        },
        RegisteredController {
            schema: schemas("execute"),
            handler: handle_execute,
        },
        RegisteredController {
            schema: schemas("list_github_repos"),
            handler: handle_list_github_repos,
        },
        RegisteredController {
            schema: schemas("create_trigger"),
            handler: handle_create_trigger,
        },
        RegisteredController {
            schema: schemas("get_user_profile"),
            handler: handle_get_user_profile,
        },
        RegisteredController {
            schema: schemas("refresh_all_identities"),
            handler: handle_refresh_all_identities,
        },
        RegisteredController {
            schema: schemas("sync"),
            handler: handle_sync,
        },
        RegisteredController {
            schema: schemas("list_trigger_history"),
            handler: handle_list_trigger_history,
        },
        RegisteredController {
            schema: schemas("get_user_scopes"),
            handler: handle_get_user_scopes,
        },
        RegisteredController {
            schema: schemas("set_user_scopes"),
            handler: handle_set_user_scopes,
        },
        RegisteredController {
            schema: schemas("list_available_triggers"),
            handler: handle_list_available_triggers,
        },
        RegisteredController {
            schema: schemas("list_triggers"),
            handler: handle_list_triggers,
        },
        RegisteredController {
            schema: schemas("enable_trigger"),
            handler: handle_enable_trigger,
        },
        RegisteredController {
            schema: schemas("disable_trigger"),
            handler: handle_disable_trigger,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_toolkits" => ControllerSchema {
            namespace: "composio",
            function: "list_toolkits",
            description: "List the Composio toolkits currently enabled on the backend allowlist.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "toolkits",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Toolkit slugs enabled by the backend (e.g. gmail, notion).",
                required: true,
            }],
        },
        "list_connections" => ControllerSchema {
            namespace: "composio",
            function: "list_connections",
            description:
                "List the caller's active Composio OAuth connections filtered to the allowlist.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "connections",
                ty: TypeSchema::Json,
                comment: "Array of {id, toolkit, status, createdAt} objects.",
                required: true,
            }],
        },
        "authorize" => ControllerSchema {
            namespace: "composio",
            function: "authorize",
            description: "Begin an OAuth handoff for a toolkit and return the hosted connect URL.",
            inputs: vec![
                FieldSchema {
                    name: "toolkit",
                    ty: TypeSchema::String,
                    comment: "Toolkit slug to authorize (must be in the backend allowlist).",
                    required: true,
                },
                FieldSchema {
                    name: "extra_params",
                    ty: TypeSchema::Json,
                    comment: "Optional JSON object of additional auth fields forwarded to Composio \
                              (e.g. {\"waba_id\": \"...\") for toolkits that require them). \
                              Reserved keys (toolkit, toolkit_version, auth, client_id) are rejected.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "connectUrl",
                    ty: TypeSchema::String,
                    comment: "Composio-hosted OAuth URL to open in a browser.",
                    required: true,
                },
                FieldSchema {
                    name: "connectionId",
                    ty: TypeSchema::String,
                    comment: "New Composio connection id created by this authorize call.",
                    required: true,
                },
            ],
        },
        "delete_connection" => ControllerSchema {
            namespace: "composio",
            function: "delete_connection",
            description: "Delete a Composio connection owned by the caller.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::String,
                comment: "Identifier of the connection to delete.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "deleted",
                ty: TypeSchema::Bool,
                comment: "True when the backend confirmed the deletion.",
                required: true,
            }],
        },
        "list_tools" => ControllerSchema {
            namespace: "composio",
            function: "list_tools",
            description:
                "List OpenAI-function-calling tool schemas for one or more Composio toolkits.",
            inputs: vec![FieldSchema {
                name: "toolkits",
                ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                comment: "Optional list of toolkit slugs to filter by. Omit to get all.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Json,
                comment: "Array of OpenAI function-calling tool schemas.",
                required: true,
            }],
        },
        "execute" => ControllerSchema {
            namespace: "composio",
            function: "execute",
            description: "Execute a Composio action (tool slug) against a connected account.",
            inputs: vec![
                FieldSchema {
                    name: "tool",
                    ty: TypeSchema::String,
                    comment: "Composio action slug, e.g. GMAIL_SEND_EMAIL.",
                    required: true,
                },
                FieldSchema {
                    name: "arguments",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Tool-specific arguments conforming to the tool's JSON schema.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Execution envelope: { data, successful, error?, costUsd }.",
                required: true,
            }],
        },
        "list_github_repos" => ControllerSchema {
            namespace: "composio",
            function: "list_github_repos",
            description:
                "List repositories available through the caller's authorized GitHub Composio connection.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment:
                    "Optional GitHub connection id. If omitted, backend picks the first active GitHub connection.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Payload: { connectionId, repositories:[{ owner, repo, fullName, ... }] }.",
                required: true,
            }],
        },
        "create_trigger" => ControllerSchema {
            namespace: "composio",
            function: "create_trigger",
            description:
                "Create a Composio trigger instance for a connected account. For GitHub triggers, pass owner/repo in trigger_config.",
            inputs: vec![
                FieldSchema {
                    name: "slug",
                    ty: TypeSchema::String,
                    comment: "Trigger slug, e.g. GITHUB_PULL_REQUEST_EVENT.",
                    required: true,
                },
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional connected account id. Backend resolves from slug toolkit when omitted.",
                    required: false,
                },
                FieldSchema {
                    name: "trigger_config",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment:
                        "Trigger config object. For GitHub, include owner/repo or repoFullName.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Payload: { triggerId, status? }.",
                required: true,
            }],
        },
        "get_user_profile" => ControllerSchema {
            namespace: "composio",
            function: "get_user_profile",
            description:
                "Fetch a normalized user profile for a Composio connection by dispatching to \
                 the toolkit's native provider implementation.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::String,
                comment: "Composio connection id (from list_connections / authorize).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "profile",
                ty: TypeSchema::Json,
                comment: "Normalized profile: { toolkit, connectionId, displayName?, email?, \
                          username?, avatarUrl?, extras }.",
                required: true,
            }],
        },
        "refresh_all_identities" => ControllerSchema {
            namespace: "composio",
            function: "refresh_all_identities",
            description:
                "Re-fetch user profile for every active Composio connection and persist as \
                 IdentityKind-tagged rows in user_profile (#1365). Best-effort per connection \
                 — failures don't abort the others.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Json,
                comment: "{ refreshed, failed, skippedNoProvider, skippedInactive, \
                          rowsWritten } — aggregate counts; per-connection trail in envelope \
                          messages.",
                required: true,
            }],
        },
        "sync" => ControllerSchema {
            namespace: "composio",
            function: "sync",
            description:
                "Run a sync pass for a Composio connection by dispatching to the toolkit's \
                 native provider implementation. Persists results into the memory layer.",
            inputs: vec![
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::String,
                    comment: "Composio connection id (from list_connections / authorize).",
                    required: true,
                },
                FieldSchema {
                    name: "reason",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Optional reason: 'manual' (default), 'periodic', 'connection_created'.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "outcome",
                ty: TypeSchema::Json,
                comment: "SyncOutcome: { toolkit, connectionId, reason, itemsIngested, \
                          startedAtMs, finishedAtMs, summary, details }.",
                required: true,
            }],
        },
        "list_trigger_history" => ControllerSchema {
            namespace: "composio",
            function: "list_trigger_history",
            description:
                "List recent ComposeIO trigger events archived by the core and report the daily JSONL archive paths.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of archived trigger events to return.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Trigger history payload: { archive_dir, current_day_file, entries }.",
                required: true,
            }],
        },
        "get_user_scopes" => ControllerSchema {
            namespace: "composio",
            function: "get_user_scopes",
            description:
                "Read the per-toolkit user scope preference (read/write/admin) used to gate \
                 composio_execute. Defaults to {read:true, write:true, admin:false} when no \
                 pref is stored.",
            inputs: vec![FieldSchema {
                name: "toolkit",
                ty: TypeSchema::String,
                comment: "Toolkit slug, e.g. 'gmail' or 'notion'.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "pref",
                ty: TypeSchema::Json,
                comment: "Scope pref: { read: bool, write: bool, admin: bool }.",
                required: true,
            }],
        },
        "set_user_scopes" => ControllerSchema {
            namespace: "composio",
            function: "set_user_scopes",
            description:
                "Persist a per-toolkit user scope preference. The agent will only be able to \
                 invoke composio actions whose classified scope is enabled here.",
            inputs: vec![
                FieldSchema {
                    name: "toolkit",
                    ty: TypeSchema::String,
                    comment: "Toolkit slug, e.g. 'gmail' or 'notion'.",
                    required: true,
                },
                FieldSchema {
                    name: "read",
                    ty: TypeSchema::Bool,
                    comment: "Allow read-classified actions (GET / FETCH / LIST / SEARCH).",
                    required: true,
                },
                FieldSchema {
                    name: "write",
                    ty: TypeSchema::Bool,
                    comment: "Allow write-classified actions (SEND / CREATE / UPDATE).",
                    required: true,
                },
                FieldSchema {
                    name: "admin",
                    ty: TypeSchema::Bool,
                    comment: "Allow admin-classified actions (DELETE / TRASH / SHARE).",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "pref",
                ty: TypeSchema::Json,
                comment: "Stored scope pref: { read, write, admin }.",
                required: true,
            }],
        },
        "list_available_triggers" => ControllerSchema {
            namespace: "composio",
            function: "list_available_triggers",
            description:
                "List the catalog of triggers the caller can enable for a toolkit. \
                 For GitHub, pass `connection_id` to fan out into per-repo entries.",
            inputs: vec![
                FieldSchema {
                    name: "toolkit",
                    ty: TypeSchema::String,
                    comment: "Toolkit slug, e.g. 'gmail' or 'github'.",
                    required: true,
                },
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Optional connection id. Optional for most toolkits; pass when \
                         requesting GitHub per-repo catalog entries.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "triggers",
                ty: TypeSchema::Json,
                comment:
                    "Array of {slug, scope, defaultConfig?, requiredConfigKeys?, repo?}.",
                required: true,
            }],
        },
        "list_triggers" => ControllerSchema {
            namespace: "composio",
            function: "list_triggers",
            description: "List the user's currently enabled Composio triggers.",
            inputs: vec![FieldSchema {
                name: "toolkit",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional toolkit slug to filter by.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "triggers",
                ty: TypeSchema::Json,
                comment:
                    "Array of {id, slug, toolkit, connectionId, triggerConfig?, state?}.",
                required: true,
            }],
        },
        "enable_trigger" => ControllerSchema {
            namespace: "composio",
            function: "enable_trigger",
            description:
                "Enable a single Composio trigger on a connection the caller owns.",
            inputs: vec![
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::String,
                    comment: "Connection id to attach the trigger to.",
                    required: true,
                },
                FieldSchema {
                    name: "slug",
                    ty: TypeSchema::String,
                    comment: "Trigger slug, e.g. 'GMAIL_NEW_GMAIL_MESSAGE'.",
                    required: true,
                },
                FieldSchema {
                    name: "trigger_config",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional trigger config object.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Payload: {triggerId, slug, connectionId}.",
                required: true,
            }],
        },
        "disable_trigger" => ControllerSchema {
            namespace: "composio",
            function: "disable_trigger",
            description: "Disable (delete) a Composio trigger owned by the caller.",
            inputs: vec![FieldSchema {
                name: "trigger_id",
                ty: TypeSchema::String,
                comment: "Identifier of the trigger to delete.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "deleted",
                ty: TypeSchema::Bool,
                comment: "True when the backend confirmed deletion.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "composio",
            function: "unknown",
            description: "Unknown composio controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

// ── Handlers ────────────────────────────────────────────────────────

fn handle_list_toolkits(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_list_toolkits(&config).await?)
    })
}

fn handle_list_connections(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_list_connections(&config).await?)
    })
}

fn handle_authorize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let toolkit = read_required_non_empty(&params, "toolkit")?;
        let extra_params = params.get("extra_params").cloned();
        to_json(super::ops::composio_authorize(&config, &toolkit, extra_params).await?)
    })
}

fn handle_delete_connection(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(super::ops::composio_delete_connection(&config, &connection_id).await?)
    })
}

fn handle_list_tools(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let toolkits = read_optional::<Vec<String>>(&params, "toolkits")?;
        to_json(super::ops::composio_list_tools(&config, toolkits).await?)
    })
}

fn handle_execute(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let tool = read_required_non_empty(&params, "tool")?;
        let arguments = read_optional::<Value>(&params, "arguments")?;
        to_json(super::ops::composio_execute(&config, &tool, arguments).await?)
    })
}

fn handle_list_github_repos(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: ListGithubReposParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(super::ops::composio_list_github_repos(&config, payload.connection_id).await?)
    })
}

fn handle_create_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: CreateTriggerParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let slug = payload.slug.trim();
        if slug.is_empty() {
            return Err("invalid params: 'slug' must not be empty".to_string());
        }
        to_json(
            super::ops::composio_create_trigger(
                &config,
                slug,
                payload.connection_id,
                payload.trigger_config,
            )
            .await?,
        )
    })
}

fn handle_list_trigger_history(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: TriggerHistoryParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(super::ops::composio_list_trigger_history(&config, payload.limit).await?)
    })
}

fn handle_get_user_profile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(super::ops::composio_get_user_profile(&config, &connection_id).await?)
    })
}

fn handle_refresh_all_identities(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_refresh_all_identities(&config).await?)
    })
}

fn handle_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        let reason = read_optional::<String>(&params, "reason")?;
        to_json(super::ops::composio_sync(&config, &connection_id, reason).await?)
    })
}

fn handle_get_user_scopes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let toolkit = match read_required_non_empty(&params, "toolkit") {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    method = "composio.get_user_scopes",
                    error = %e,
                    "[composio:scopes] missing required `toolkit` param"
                );
                return Err(e);
            }
        };
        tracing::debug!(
            method = "composio.get_user_scopes",
            toolkit = %toolkit,
            "[composio:scopes] handler entry"
        );
        let pref = super::providers::user_scopes::load_or_default(&toolkit).await;
        tracing::debug!(
            method = "composio.get_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler exit"
        );
        to_json(crate::rpc::RpcOutcome::new(pref, vec![]))
    })
}

fn handle_set_user_scopes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let toolkit = match read_required_non_empty(&params, "toolkit") {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    method = "composio.set_user_scopes",
                    error = %e,
                    "[composio:scopes] missing required `toolkit` param"
                );
                return Err(e);
            }
        };
        let read: bool = read_required(&params, "read")?;
        let write: bool = read_required(&params, "write")?;
        let admin: bool = read_required(&params, "admin")?;
        let pref = super::providers::UserScopePref { read, write, admin };
        tracing::debug!(
            method = "composio.set_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler entry"
        );
        let memory = match crate::openhuman::memory::global::client_if_ready() {
            Some(m) => m,
            None => {
                tracing::error!(
                    method = "composio.set_user_scopes",
                    toolkit = %toolkit,
                    "[composio:scopes] memory client not initialised — cannot persist pref"
                );
                return Err("memory client not initialised".to_string());
            }
        };
        if let Err(e) = super::providers::user_scopes::save(&memory, &toolkit, pref).await {
            tracing::error!(
                method = "composio.set_user_scopes",
                toolkit = %toolkit,
                error = %e,
                "[composio:scopes] save failed"
            );
            return Err(e);
        }
        tracing::debug!(
            method = "composio.set_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler exit"
        );
        to_json(crate::rpc::RpcOutcome::new(pref, vec![]))
    })
}

fn handle_list_available_triggers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: ListAvailableTriggersParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let toolkit = payload.toolkit.trim();
        if toolkit.is_empty() {
            return Err("invalid params: 'toolkit' must not be empty".to_string());
        }
        to_json(
            super::ops::composio_list_available_triggers(&config, toolkit, payload.connection_id)
                .await?,
        )
    })
}

fn handle_list_triggers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: ListTriggersParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(super::ops::composio_list_triggers(&config, payload.toolkit).await?)
    })
}

fn handle_enable_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: EnableTriggerParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let connection_id = payload.connection_id.trim();
        let slug = payload.slug.trim();
        if connection_id.is_empty() {
            return Err("invalid params: 'connection_id' must not be empty".to_string());
        }
        if slug.is_empty() {
            return Err("invalid params: 'slug' must not be empty".to_string());
        }
        to_json(
            super::ops::composio_enable_trigger(
                &config,
                connection_id,
                slug,
                payload.trigger_config,
            )
            .await?,
        )
    })
}

fn handle_disable_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let trigger_id = read_required_non_empty(&params, "trigger_id")?;
        to_json(super::ops::composio_disable_trigger(&config, &trigger_id).await?)
    })
}

// ── Param helpers ───────────────────────────────────────────────────

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

/// Read a required `String` parameter and reject blank / whitespace-only
/// input at the RPC boundary instead of letting it reach the backend.
/// Returns the trimmed value.
fn read_required_non_empty(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    let raw = read_required::<String>(params, key)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("'{key}' must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
