//! Controller schemas and handler dispatch for the MCP clients domain.
//!
//! Every `schemas(function)` match arm defines the RPC method's input/output
//! shape. Every `handle_*` function deserialises params and delegates to
//! `ops.rs`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

// ── Schema registry ──────────────────────────────────────────────────────────

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("registry_search"),
        schemas("registry_get"),
        schemas("installed_list"),
        schemas("install"),
        schemas("uninstall"),
        schemas("connect"),
        schemas("disconnect"),
        schemas("status"),
        schemas("tool_call"),
        schemas("config_assist"),
        // Setup-agent surface (mcp_setup namespace, lives in setup_ops.rs).
        setup_schemas("search"),
        setup_schemas("get"),
        setup_schemas("request_secret"),
        setup_schemas("submit_secret"),
        setup_schemas("test_connection"),
        setup_schemas("install_and_connect"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("registry_search"),
            handler: handle_registry_search,
        },
        RegisteredController {
            schema: schemas("registry_get"),
            handler: handle_registry_get,
        },
        RegisteredController {
            schema: schemas("installed_list"),
            handler: handle_installed_list,
        },
        RegisteredController {
            schema: schemas("install"),
            handler: handle_install,
        },
        RegisteredController {
            schema: schemas("uninstall"),
            handler: handle_uninstall,
        },
        RegisteredController {
            schema: schemas("connect"),
            handler: handle_connect,
        },
        RegisteredController {
            schema: schemas("disconnect"),
            handler: handle_disconnect,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("tool_call"),
            handler: handle_tool_call,
        },
        RegisteredController {
            schema: schemas("config_assist"),
            handler: handle_config_assist,
        },
        RegisteredController {
            schema: setup_schemas("search"),
            handler: handle_setup_search,
        },
        RegisteredController {
            schema: setup_schemas("get"),
            handler: handle_setup_get,
        },
        RegisteredController {
            schema: setup_schemas("request_secret"),
            handler: handle_setup_request_secret,
        },
        RegisteredController {
            schema: setup_schemas("submit_secret"),
            handler: handle_setup_submit_secret,
        },
        RegisteredController {
            schema: setup_schemas("test_connection"),
            handler: handle_setup_test_connection,
        },
        RegisteredController {
            schema: setup_schemas("install_and_connect"),
            handler: handle_setup_install_and_connect,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "registry_search" => ControllerSchema {
            namespace: "mcp_clients",
            function: "registry_search",
            description: "Search the Smithery.ai MCP server registry.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Free-text search query.",
                    required: false,
                },
                FieldSchema {
                    name: "page",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "1-based page number (default: 1).",
                    required: false,
                },
                FieldSchema {
                    name: "page_size",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Results per page (default: 20).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "servers",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("SmitheryServerSummary"))),
                    comment: "Matching server summaries from the registry.",
                    required: true,
                },
                FieldSchema {
                    name: "page",
                    ty: TypeSchema::U64,
                    comment: "Current page number.",
                    required: true,
                },
                FieldSchema {
                    name: "total_pages",
                    ty: TypeSchema::U64,
                    comment: "Total number of pages available.",
                    required: true,
                },
            ],
        },

        "registry_get" => ControllerSchema {
            namespace: "mcp_clients",
            function: "registry_get",
            description: "Fetch full details for one MCP server from the Smithery registry.",
            inputs: vec![FieldSchema {
                name: "qualified_name",
                ty: TypeSchema::String,
                comment: "Registry qualified name, e.g. `@modelcontextprotocol/server-filesystem`.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "server",
                ty: TypeSchema::Ref("SmitheryServerDetail"),
                comment: "Full server detail including connection specs.",
                required: true,
            }],
        },

        "installed_list" => ControllerSchema {
            namespace: "mcp_clients",
            function: "installed_list",
            description: "List all locally installed MCP servers.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "installed",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("InstalledServer"))),
                comment: "Installed server records (env values omitted).",
                required: true,
            }],
        },

        "install" => ControllerSchema {
            namespace: "mcp_clients",
            function: "install",
            description: "Install an MCP server from the Smithery registry.",
            inputs: vec![
                FieldSchema {
                    name: "qualified_name",
                    ty: TypeSchema::String,
                    comment: "Registry qualified name.",
                    required: true,
                },
                FieldSchema {
                    name: "env",
                    ty: TypeSchema::Map(Box::new(TypeSchema::String)),
                    comment: "Environment variable values required by the server. Values are stored encrypted and never returned.",
                    required: true,
                },
                FieldSchema {
                    name: "config",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional JSON configuration blob.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "server",
                ty: TypeSchema::Ref("InstalledServer"),
                comment: "The newly installed server record.",
                required: true,
            }],
        },

        "uninstall" => ControllerSchema {
            namespace: "mcp_clients",
            function: "uninstall",
            description: "Uninstall a locally installed MCP server.",
            inputs: vec![FieldSchema {
                name: "server_id",
                ty: TypeSchema::String,
                comment: "UUID of the server to remove.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "The server id that was targeted.",
                    required: true,
                },
                FieldSchema {
                    name: "removed",
                    ty: TypeSchema::Bool,
                    comment: "True when the server was actually removed.",
                    required: true,
                },
            ],
        },

        "connect" => ControllerSchema {
            namespace: "mcp_clients",
            function: "connect",
            description: "Spawn the MCP server subprocess and run the initialize handshake.",
            inputs: vec![FieldSchema {
                name: "server_id",
                ty: TypeSchema::String,
                comment: "UUID of the installed server to connect.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "Connected server id.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::Enum {
                        variants: vec!["connected"],
                    },
                    comment: "Always `connected` on success.",
                    required: true,
                },
                FieldSchema {
                    name: "tools",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("McpTool"))),
                    comment: "Tools exposed by the connected server.",
                    required: true,
                },
            ],
        },

        "disconnect" => ControllerSchema {
            namespace: "mcp_clients",
            function: "disconnect",
            description: "Disconnect a running MCP server and stop its process.",
            inputs: vec![FieldSchema {
                name: "server_id",
                ty: TypeSchema::String,
                comment: "UUID of the server to disconnect.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "Disconnected server id.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::Enum {
                        variants: vec!["disconnected"],
                    },
                    comment: "Always `disconnected` on success.",
                    required: true,
                },
            ],
        },

        "status" => ControllerSchema {
            namespace: "mcp_clients",
            function: "status",
            description: "Return connection status for all installed MCP servers.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "servers",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ConnStatus"))),
                comment: "Per-server connection status summaries.",
                required: true,
            }],
        },

        "tool_call" => ControllerSchema {
            namespace: "mcp_clients",
            function: "tool_call",
            description: "Invoke a tool on a connected MCP server.",
            inputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "UUID of the connected server.",
                    required: true,
                },
                FieldSchema {
                    name: "tool_name",
                    ty: TypeSchema::String,
                    comment: "Name of the tool to call.",
                    required: true,
                },
                FieldSchema {
                    name: "arguments",
                    ty: TypeSchema::Json,
                    comment: "Tool arguments as a JSON value.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "result",
                    ty: TypeSchema::Json,
                    comment: "Tool result value.",
                    required: true,
                },
                FieldSchema {
                    name: "is_error",
                    ty: TypeSchema::Bool,
                    comment: "True when the tool returned an error.",
                    required: true,
                },
            ],
        },

        "config_assist" => ControllerSchema {
            namespace: "mcp_clients",
            function: "config_assist",
            description: "AI assistant that helps configure an MCP server's required env vars.",
            inputs: vec![
                FieldSchema {
                    name: "qualified_name",
                    ty: TypeSchema::String,
                    comment: "Registry qualified name of the server being configured.",
                    required: true,
                },
                FieldSchema {
                    name: "user_message",
                    ty: TypeSchema::String,
                    comment: "User's question or reply.",
                    required: true,
                },
                FieldSchema {
                    name: "history",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::Ref(
                        "ChatTurn",
                    ))))),
                    comment: "Prior conversation turns `[{role, content}]`.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "reply",
                    ty: TypeSchema::String,
                    comment: "Assistant reply (markdown).",
                    required: true,
                },
                FieldSchema {
                    name: "suggested_env",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Map(Box::new(TypeSchema::String)))),
                    comment: "Env vars extracted from the user's message, if any.",
                    required: false,
                },
            ],
        },

        // Handled by setup_schemas() — surface a clearer error rather than
        // falling through to the generic unknown sink.
        "setup_search"
        | "setup_get"
        | "setup_request_secret"
        | "setup_submit_secret"
        | "setup_test_connection"
        | "setup_install_and_connect" => setup_schemas(function.trim_start_matches("setup_")),

        _other => ControllerSchema {
            namespace: "mcp_clients",
            function: "unknown",
            description: "Unknown mcp_clients controller function.",
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

// ── Handler implementations ──────────────────────────────────────────────────

fn handle_registry_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let query = read_optional_string(&params, "query")?;
        let page = read_optional_u32(&params, "page")?;
        let page_size = read_optional_u32(&params, "page_size")?;
        to_json(
            crate::openhuman::mcp_registry::ops::mcp_clients_registry_search(
                &config, query, page, page_size,
            )
            .await?,
        )
    })
}

fn handle_registry_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let qualified_name = read_required::<String>(&params, "qualified_name")?;
        to_json(
            crate::openhuman::mcp_registry::ops::mcp_clients_registry_get(&config, qualified_name)
                .await?,
        )
    })
}

fn handle_installed_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::mcp_registry::ops::mcp_clients_installed_list(&config).await?)
    })
}

fn handle_install(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let qualified_name = read_required::<String>(&params, "qualified_name")?;
        let env = read_required::<std::collections::HashMap<String, String>>(&params, "env")?;
        let config_value = read_optional_json(&params, "config")?;
        to_json(
            crate::openhuman::mcp_registry::ops::mcp_clients_install(
                &config,
                qualified_name,
                env,
                config_value,
            )
            .await?,
        )
    })
}

fn handle_uninstall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(
            crate::openhuman::mcp_registry::ops::mcp_clients_uninstall(&config, server_id).await?,
        )
    })
}

fn handle_connect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(crate::openhuman::mcp_registry::ops::mcp_clients_connect(&config, server_id).await?)
    })
}

fn handle_disconnect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(crate::openhuman::mcp_registry::ops::mcp_clients_disconnect(server_id).await?)
    })
}

fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::mcp_registry::ops::mcp_clients_status(&config).await?)
    })
}

fn handle_tool_call(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let server_id = read_required::<String>(&params, "server_id")?;
        let tool_name = read_required::<String>(&params, "tool_name")?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        to_json(
            crate::openhuman::mcp_registry::ops::mcp_clients_tool_call(
                server_id, tool_name, arguments,
            )
            .await?,
        )
    })
}

fn handle_config_assist(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let qualified_name = read_required::<String>(&params, "qualified_name")?;
        let user_message = read_required::<String>(&params, "user_message")?;
        let history = read_optional::<Vec<crate::openhuman::mcp_registry::types::ChatTurn>>(
            &params, "history",
        )?;
        to_json(
            crate::openhuman::mcp_registry::ops::mcp_clients_config_assist(
                &config,
                qualified_name,
                user_message,
                history,
            )
            .await?,
        )
    })
}

// ── mcp_setup_* schemas + handlers ────────────────────────────────────────────

/// All setup-agent schemas under the `mcp_setup` RPC namespace. Kept in a
/// separate function so the setup surface can evolve independently of the
/// existing `mcp_clients_*` controllers.
pub fn setup_schemas(function: &str) -> ControllerSchema {
    match function {
        "search" => ControllerSchema {
            namespace: "mcp_setup",
            function: "search",
            description: "Search all enabled MCP registries (Smithery + official).",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Free-text search query.",
                    required: false,
                },
                FieldSchema {
                    name: "page",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "1-based page number (default: 1).",
                    required: false,
                },
                FieldSchema {
                    name: "page_size",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Results per page (default: 20).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "servers",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("SmitheryServerSummary"))),
                    comment: "Merged summaries; each row tagged with its `source` (`smithery` | `mcp_official`).",
                    required: true,
                },
                FieldSchema {
                    name: "page",
                    ty: TypeSchema::U64,
                    comment: "Current page number.",
                    required: true,
                },
                FieldSchema {
                    name: "total_pages",
                    ty: TypeSchema::U64,
                    comment: "Upper-bound page count across registries.",
                    required: true,
                },
            ],
        },
        "get" => ControllerSchema {
            namespace: "mcp_setup",
            function: "get",
            description: "Fetch full details for one server. Adds `required_env_keys` derived from the connection schema.",
            inputs: vec![FieldSchema {
                name: "qualified_name",
                ty: TypeSchema::String,
                comment: "Registry qualified name. May be prefixed with `<source>::` to pin a registry.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "server",
                ty: TypeSchema::Ref("SmitheryServerDetail"),
                comment: "Full detail with `required_env_keys` injected.",
                required: true,
            }],
        },
        "request_secret" => ControllerSchema {
            namespace: "mcp_setup",
            function: "request_secret",
            description: "Ask the user out-of-band for a secret value. Blocks until the UI submits via `submit_secret` (5-minute timeout). Returns an opaque ref; the raw value never enters the agent's context.",
            inputs: vec![
                FieldSchema {
                    name: "key_name",
                    ty: TypeSchema::String,
                    comment: "Display name of the env var (e.g. `NOTION_API_KEY`).",
                    required: true,
                },
                FieldSchema {
                    name: "prompt",
                    ty: TypeSchema::String,
                    comment: "Plain-English instruction shown to the user in the native input box.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "ref",
                    ty: TypeSchema::String,
                    comment: "Opaque handle like `secret://<hex>`. Pass back via `test_connection` / `install_and_connect`.",
                    required: true,
                },
                FieldSchema {
                    name: "key_name",
                    ty: TypeSchema::String,
                    comment: "Echoed key name.",
                    required: true,
                },
            ],
        },
        "submit_secret" => ControllerSchema {
            namespace: "mcp_setup",
            function: "submit_secret",
            description: "UI-side: fulfill a pending `request_secret` with the user-entered value. Not intended for agent use.",
            inputs: vec![
                FieldSchema {
                    name: "ref_id",
                    ty: TypeSchema::String,
                    comment: "The `secret://<hex>` ref returned by `request_secret`.",
                    required: true,
                },
                FieldSchema {
                    name: "value",
                    ty: TypeSchema::String,
                    comment: "Raw secret value. NEVER log this.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "ref",
                    ty: TypeSchema::String,
                    comment: "Echoed ref.",
                    required: true,
                },
                FieldSchema {
                    name: "fulfilled",
                    ty: TypeSchema::Bool,
                    comment: "True on success.",
                    required: true,
                },
            ],
        },
        "test_connection" => ControllerSchema {
            namespace: "mcp_setup",
            function: "test_connection",
            description: "Dry-run install: spawn a candidate server in a scratch process with the supplied secret refs, list its tools, tear down. Nothing persisted.",
            inputs: vec![
                FieldSchema {
                    name: "qualified_name",
                    ty: TypeSchema::String,
                    comment: "Registry qualified name.",
                    required: true,
                },
                FieldSchema {
                    name: "env_refs",
                    ty: TypeSchema::Map(Box::new(TypeSchema::String)),
                    comment: "Map `{ENV_KEY: secret://<hex>}` produced by `request_secret`.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "ok",
                    ty: TypeSchema::Bool,
                    comment: "True if initialize + tools/list succeeded.",
                    required: true,
                },
                FieldSchema {
                    name: "tools",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::Ref(
                        "McpRemoteTool",
                    ))))),
                    comment: "Tools advertised by the candidate. Present iff `ok`.",
                    required: false,
                },
                FieldSchema {
                    name: "error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Error string. Present iff `ok` is false.",
                    required: false,
                },
            ],
        },
        "install_and_connect" => ControllerSchema {
            namespace: "mcp_setup",
            function: "install_and_connect",
            description: "Commit: persist the install + secrets (consuming the refs), then connect immediately and return the tool list.",
            inputs: vec![
                FieldSchema {
                    name: "qualified_name",
                    ty: TypeSchema::String,
                    comment: "Registry qualified name.",
                    required: true,
                },
                FieldSchema {
                    name: "env_refs",
                    ty: TypeSchema::Map(Box::new(TypeSchema::String)),
                    comment: "Map `{ENV_KEY: secret://<hex>}`. Refs are consumed (removed from the in-memory map) on success.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "Freshly-minted server UUID.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::String,
                    comment: "`connected` or `installed_disconnected` (install succeeded, connect failed).",
                    required: true,
                },
                FieldSchema {
                    name: "tools",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::Ref(
                        "McpTool",
                    ))))),
                    comment: "Tool list iff `status == connected`.",
                    required: false,
                },
                FieldSchema {
                    name: "error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Connect error iff `status != connected`.",
                    required: false,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "mcp_setup",
            function: "unknown",
            description: "Unknown mcp_setup controller function.",
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

fn handle_setup_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let query = read_optional_string(&params, "query")?;
        let page = read_optional_u32(&params, "page")?;
        let page_size = read_optional_u32(&params, "page_size")?;
        to_json(
            crate::openhuman::mcp_registry::setup_ops::mcp_setup_search(
                &config, query, page, page_size,
            )
            .await?,
        )
    })
}

fn handle_setup_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let qualified_name = read_required::<String>(&params, "qualified_name")?;
        to_json(
            crate::openhuman::mcp_registry::setup_ops::mcp_setup_get(&config, qualified_name)
                .await?,
        )
    })
}

fn handle_setup_request_secret(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let key_name = read_required::<String>(&params, "key_name")?;
        let prompt = read_required::<String>(&params, "prompt")?;
        to_json(
            crate::openhuman::mcp_registry::setup_ops::mcp_setup_request_secret(key_name, prompt)
                .await?,
        )
    })
}

fn handle_setup_submit_secret(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let ref_id = read_required::<String>(&params, "ref_id")?;
        let value = read_required::<String>(&params, "value")?;
        to_json(
            crate::openhuman::mcp_registry::setup_ops::mcp_setup_submit_secret(ref_id, value)
                .await?,
        )
    })
}

fn handle_setup_test_connection(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let qualified_name = read_required::<String>(&params, "qualified_name")?;
        let env_refs =
            read_required::<std::collections::HashMap<String, String>>(&params, "env_refs")?;
        to_json(
            crate::openhuman::mcp_registry::setup_ops::mcp_setup_test_connection(
                &config,
                qualified_name,
                env_refs,
            )
            .await?,
        )
    })
}

fn handle_setup_install_and_connect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let qualified_name = read_required::<String>(&params, "qualified_name")?;
        let env_refs =
            read_required::<std::collections::HashMap<String, String>>(&params, "env_refs")?;
        to_json(
            crate::openhuman::mcp_registry::setup_ops::mcp_setup_install_and_connect(
                &config,
                qualified_name,
                env_refs,
            )
            .await?,
        )
    })
}

// ── Param helpers ─────────────────────────────────────────────────────────────

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn read_optional_string(params: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    read_optional::<String>(params, key)
}

fn read_optional_u32(params: &Map<String, Value>, key: &str) -> Result<Option<u32>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected u32")),
        Some(other) => Err(format!(
            "invalid '{key}': expected number, got {}",
            type_name(other)
        )),
    }
}

fn read_optional_json(params: &Map<String, Value>, key: &str) -> Result<Option<Value>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(v.clone())),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
