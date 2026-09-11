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
        schemas("update_env"),
        schemas("uninstall"),
        schemas("detect_auth"),
        schemas("oauth_begin"),
        schemas("connect"),
        schemas("disconnect"),
        schemas("status"),
        schemas("tool_call"),
        schemas("config_assist"),
        schemas("registry_settings_get"),
        schemas("registry_settings_set"),
        schemas("set_enabled"),
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
            schema: schemas("update_env"),
            handler: handle_update_env,
        },
        RegisteredController {
            schema: schemas("uninstall"),
            handler: handle_uninstall,
        },
        RegisteredController {
            schema: schemas("detect_auth"),
            handler: handle_detect_auth,
        },
        RegisteredController {
            schema: schemas("oauth_begin"),
            handler: handle_oauth_begin,
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
            schema: schemas("registry_settings_get"),
            handler: handle_registry_settings_get,
        },
        RegisteredController {
            schema: schemas("registry_settings_set"),
            handler: handle_registry_settings_set,
        },
        RegisteredController {
            schema: schemas("set_enabled"),
            handler: handle_set_enabled,
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
                    name: "transport",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Transport filter: \"stdio\", \"hosted\", or \"all\"/omitted.",
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

        "update_env" => ControllerSchema {
            namespace: "mcp_clients",
            function: "update_env",
            description: "Replace the stored env values for an installed server and reconnect so the new credentials take effect (reconfigure / rotate keys without reinstalling).",
            inputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "UUID of the installed server to reconfigure.",
                    required: true,
                },
                FieldSchema {
                    name: "env",
                    ty: TypeSchema::Map(Box::new(TypeSchema::String)),
                    comment: "Replacement environment variable values. Stored encrypted and never returned.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "The reconfigured server id.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::Enum {
                        variants: vec!["connected", "disconnected"],
                    },
                    comment: "`connected` if the reconnect succeeded, `disconnected` if env was saved but reconnect failed.",
                    required: true,
                },
                FieldSchema {
                    name: "env_keys",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Env key names after the update (values omitted).",
                    required: true,
                },
                FieldSchema {
                    name: "tools",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("McpTool"))),
                    comment: "Tools exposed after reconnect (present only when status=connected).",
                    required: false,
                },
            ],
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

        "detect_auth" => ControllerSchema {
            namespace: "mcp_clients",
            function: "detect_auth",
            description: "Probe a server to classify how it authenticates (none / token / oauth).",
            inputs: vec![FieldSchema {
                name: "server_id",
                ty: TypeSchema::String,
                comment: "UUID of the installed server to probe.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "kind",
                    ty: TypeSchema::Enum {
                        variants: vec!["none", "token", "oauth"],
                    },
                    comment: "`none` (open), `token` (static bearer/API key), or `oauth` (browser sign-in).",
                    required: true,
                },
                FieldSchema {
                    name: "authorization_endpoint",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "OAuth authorization endpoint, when kind is `oauth`.",
                    required: false,
                },
                FieldSchema {
                    name: "grant_types",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Grant types the authorization server supports.",
                    required: true,
                },
            ],
        },

        "oauth_begin" => ControllerSchema {
            namespace: "mcp_clients",
            function: "oauth_begin",
            description: "Begin browser OAuth: discover + dynamically register a client + PKCE, returning the authorize URL to open.",
            inputs: vec![FieldSchema {
                name: "server_id",
                ty: TypeSchema::String,
                comment: "UUID of the installed server to authenticate.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "authorize_url",
                ty: TypeSchema::String,
                comment: "Live OAuth authorize URL to open in the browser; the /oauth/mcp/callback route completes sign-in.",
                required: true,
            }],
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

        "registry_settings_get" => ControllerSchema {
            namespace: "mcp_clients",
            function: "registry_settings_get",
            description: "Report which registry credentials are configured (Smithery key, official-registry base/token). Never returns secret values — only `*_set` booleans plus the non-secret base URL override.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "smithery_api_key_set",
                    ty: TypeSchema::Bool,
                    comment: "True when a Smithery API key is set (config or env).",
                    required: true,
                },
                FieldSchema {
                    name: "mcp_official_token_set",
                    ty: TypeSchema::Bool,
                    comment: "True when an official-registry bearer token is set (config or env).",
                    required: true,
                },
                FieldSchema {
                    name: "mcp_official_base",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "User-configured official-registry base URL override (non-secret).",
                    required: false,
                },
            ],
        },

        "registry_settings_set" => ControllerSchema {
            namespace: "mcp_clients",
            function: "registry_settings_set",
            description: "Persist registry credentials. Per field: omit to leave unchanged, empty string to clear, value to set. Secrets are write-only; the response is the same non-secret snapshot as registry_settings_get.",
            inputs: vec![
                FieldSchema {
                    name: "smithery_api_key",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New Smithery API key (empty string clears).",
                    required: false,
                },
                FieldSchema {
                    name: "mcp_official_base",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New official-registry base URL override (empty string clears).",
                    required: false,
                },
                FieldSchema {
                    name: "mcp_official_token",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New official-registry bearer token (empty string clears).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "smithery_api_key_set",
                    ty: TypeSchema::Bool,
                    comment: "True when a Smithery API key is set after the update.",
                    required: true,
                },
                FieldSchema {
                    name: "mcp_official_token_set",
                    ty: TypeSchema::Bool,
                    comment: "True when an official-registry token is set after the update.",
                    required: true,
                },
                FieldSchema {
                    name: "mcp_official_base",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Official-registry base URL override after the update.",
                    required: false,
                },
            ],
        },

        "set_enabled" => ControllerSchema {
            namespace: "mcp_clients",
            function: "set_enabled",
            description: "Enable or disable an installed MCP server. Disabling auto-disconnects any live session and hides the server's tools from the agent; the install row and env values are kept so re-enabling does not require re-entry.",
            inputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "UUID of the installed server.",
                    required: true,
                },
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "Target state; `false` also disconnects.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "server_id",
                    ty: TypeSchema::String,
                    comment: "Echoed server id.",
                    required: true,
                },
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "Effective enabled state after the call.",
                    required: true,
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
