use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

use super::qjs_engine::require_engine;
use super::registry_ops;

const SOCKET_UNAVAILABLE_MSG: &str =
    "native skill runtime and socket manager are not available in this build";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        socket_schema("connect"),
        socket_schema("disconnect"),
        socket_schema("state"),
        socket_schema("emit"),
        skills_schema("registry_fetch"),
        skills_schema("search"),
        skills_schema("install"),
        skills_schema("uninstall"),
        skills_schema("list_installed"),
        skills_schema("list_available"),
        skills_schema("start"),
        skills_schema("stop"),
        // Runtime controllers
        skills_schema("status"),
        skills_schema("setup_start"),
        skills_schema("list_tools"),
        skills_schema("sync"),
        skills_schema("call_tool"),
        skills_schema("rpc"),
        skills_schema("discover"),
        skills_schema("list"),
        skills_schema("data_read"),
        skills_schema("data_write"),
        skills_schema("data_dir"),
        skills_schema("enable"),
        skills_schema("disable"),
        skills_schema("is_enabled"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        // Socket stubs (unchanged)
        RegisteredController {
            schema: socket_schema("connect"),
            handler: handle_socket_unavailable,
        },
        RegisteredController {
            schema: socket_schema("disconnect"),
            handler: handle_socket_unavailable,
        },
        RegisteredController {
            schema: socket_schema("state"),
            handler: handle_socket_unavailable,
        },
        RegisteredController {
            schema: socket_schema("emit"),
            handler: handle_socket_unavailable,
        },
        // Skills registry controllers
        RegisteredController {
            schema: skills_schema("registry_fetch"),
            handler: handle_skills_registry_fetch,
        },
        RegisteredController {
            schema: skills_schema("search"),
            handler: handle_skills_search,
        },
        RegisteredController {
            schema: skills_schema("install"),
            handler: handle_skills_install,
        },
        RegisteredController {
            schema: skills_schema("uninstall"),
            handler: handle_skills_uninstall,
        },
        RegisteredController {
            schema: skills_schema("list_installed"),
            handler: handle_skills_list_installed,
        },
        RegisteredController {
            schema: skills_schema("list_available"),
            handler: handle_skills_list_available,
        },
        RegisteredController {
            schema: skills_schema("start"),
            handler: handle_skills_start,
        },
        RegisteredController {
            schema: skills_schema("stop"),
            handler: handle_skills_stop,
        },
        // Runtime controllers
        RegisteredController {
            schema: skills_schema("status"),
            handler: handle_skills_status,
        },
        RegisteredController {
            schema: skills_schema("setup_start"),
            handler: handle_skills_setup_start,
        },
        RegisteredController {
            schema: skills_schema("list_tools"),
            handler: handle_skills_list_tools,
        },
        RegisteredController {
            schema: skills_schema("sync"),
            handler: handle_skills_sync,
        },
        RegisteredController {
            schema: skills_schema("call_tool"),
            handler: handle_skills_call_tool,
        },
        RegisteredController {
            schema: skills_schema("rpc"),
            handler: handle_skills_rpc,
        },
        RegisteredController {
            schema: skills_schema("discover"),
            handler: handle_skills_discover,
        },
        RegisteredController {
            schema: skills_schema("list"),
            handler: handle_skills_list,
        },
        RegisteredController {
            schema: skills_schema("data_read"),
            handler: handle_skills_data_read,
        },
        RegisteredController {
            schema: skills_schema("data_write"),
            handler: handle_skills_data_write,
        },
        RegisteredController {
            schema: skills_schema("data_dir"),
            handler: handle_skills_data_dir,
        },
        RegisteredController {
            schema: skills_schema("enable"),
            handler: handle_skills_enable,
        },
        RegisteredController {
            schema: skills_schema("disable"),
            handler: handle_skills_disable,
        },
        RegisteredController {
            schema: skills_schema("is_enabled"),
            handler: handle_skills_is_enabled,
        },
    ]
}

// --- Socket schemas (unchanged) ---

fn socket_schema(function: &str) -> ControllerSchema {
    match function {
        "connect" | "disconnect" | "state" | "emit" => ControllerSchema {
            namespace: "socket",
            function: match function {
                "connect" => "connect",
                "disconnect" => "disconnect",
                "state" => "state",
                _ => "emit",
            },
            description: "Skill runtime socket manager bridge.",
            inputs: vec![FieldSchema {
                name: "payload",
                ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                comment: "Socket request payload.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Socket response payload.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "socket",
            function: "unknown",
            description: "Unknown socket controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_socket_unavailable(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { Err(SOCKET_UNAVAILABLE_MSG.to_string()) })
}

// --- Skills registry schemas ---

fn skills_schema(function: &str) -> ControllerSchema {
    match function {
        "registry_fetch" => ControllerSchema {
            namespace: "skills",
            function: "registry_fetch",
            description: "Fetch the remote skill registry (cached with 1h TTL).",
            inputs: vec![FieldSchema {
                name: "force",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Force a fresh fetch, bypassing cache.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "The full registry JSON.",
                required: true,
            }],
        },
        "search" => ControllerSchema {
            namespace: "skills",
            function: "search",
            description: "Search available skills by name, description, or ID.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: true,
                },
                FieldSchema {
                    name: "category",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Filter by category: 'core' or 'third_party'.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of matching skill entries.",
                required: true,
            }],
        },
        "install" => ControllerSchema {
            namespace: "skills",
            function: "install",
            description: "Download and install a skill from the registry.",
            inputs: vec![FieldSchema {
                name: "skill_id",
                ty: TypeSchema::String,
                comment: "The skill ID to install.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Installation result.",
                required: true,
            }],
        },
        "uninstall" => ControllerSchema {
            namespace: "skills",
            function: "uninstall",
            description: "Remove an installed skill from the workspace.",
            inputs: vec![FieldSchema {
                name: "skill_id",
                ty: TypeSchema::String,
                comment: "The skill ID to uninstall.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Uninstallation result.",
                required: true,
            }],
        },
        "list_installed" => ControllerSchema {
            namespace: "skills",
            function: "list_installed",
            description: "List all locally installed skills.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of installed skill info.",
                required: true,
            }],
        },
        "list_available" => ControllerSchema {
            namespace: "skills",
            function: "list_available",
            description: "List all available skills with installed status.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of available skill entries with installed flags.",
                required: true,
            }],
        },
        // --- Runtime controllers ---
        "start" => ControllerSchema {
            namespace: "skills",
            function: "start",
            description: "Start (load and initialize) a skill by ID.",
            inputs: vec![skill_id_input("The skill ID to start.")],
            outputs: vec![FieldSchema {
                name: "skill",
                ty: TypeSchema::Json,
                comment: "Skill snapshot after start (id, name, status, tools, state).",
                required: true,
            }],
        },
        "stop" => ControllerSchema {
            namespace: "skills",
            function: "stop",
            description: "Stop a running skill by ID.",
            inputs: vec![skill_id_input("The skill ID to stop.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Stop acknowledgment.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "skills",
            function: "status",
            description: "Get the runtime status and state of a skill.",
            inputs: vec![skill_id_input("The skill ID to query.")],
            outputs: vec![FieldSchema {
                name: "skill",
                ty: TypeSchema::Json,
                comment: "Skill snapshot (id, name, status, tools, error, state).",
                required: true,
            }],
        },
        "setup_start" => ControllerSchema {
            namespace: "skills",
            function: "setup_start",
            description:
                "Trigger the setup flow for a running skill, returning the first setup step.",
            inputs: vec![skill_id_input("The skill ID to set up.")],
            outputs: vec![FieldSchema {
                name: "step",
                ty: TypeSchema::Json,
                comment:
                    "First setup step definition (fields, labels, etc.) or null if no setup needed.",
                required: true,
            }],
        },
        "list_tools" => ControllerSchema {
            namespace: "skills",
            function: "list_tools",
            description: "List all tools exposed by a running skill.",
            inputs: vec![skill_id_input("The skill ID whose tools to list.")],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of tool definitions (name, description, inputSchema).",
                required: true,
            }],
        },
        "sync" => ControllerSchema {
            namespace: "skills",
            function: "sync",
            description: "Trigger a sync (tick) on a running skill.",
            inputs: vec![skill_id_input("The skill ID to sync.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Sync acknowledgment.",
                required: true,
            }],
        },
        "call_tool" => ControllerSchema {
            namespace: "skills",
            function: "call_tool",
            description: "Call a specific tool on a running skill.",
            inputs: vec![
                skill_id_input("The skill ID that owns the tool."),
                FieldSchema {
                    name: "tool_name",
                    ty: TypeSchema::String,
                    comment: "Name of the tool to call.",
                    required: true,
                },
                FieldSchema {
                    name: "arguments",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "JSON arguments to pass to the tool.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Tool execution result (content blocks, is_error flag).",
                required: true,
            }],
        },
        "rpc" => ControllerSchema {
            namespace: "skills",
            function: "rpc",
            description: "Send an arbitrary RPC method to a running skill (e.g. oauth/complete, skill/sync).",
            inputs: vec![
                skill_id_input("The target skill ID."),
                FieldSchema {
                    name: "method",
                    ty: TypeSchema::String,
                    comment: "RPC method name (e.g. oauth/complete, skill/ping).",
                    required: true,
                },
                FieldSchema {
                    name: "params",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "JSON params to pass to the RPC handler.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "RPC handler result.",
                required: true,
            }],
        },
        "discover" => ControllerSchema {
            namespace: "skills",
            function: "discover",
            description: "Discover all available skill manifests from source and workspace directories.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of skill manifests.",
                required: true,
            }],
        },
        "list" => ControllerSchema {
            namespace: "skills",
            function: "list",
            description: "List all registered (running or stopped) skill snapshots.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of skill snapshots.",
                required: true,
            }],
        },
        "data_read" => ControllerSchema {
            namespace: "skills",
            function: "data_read",
            description: "Read a file from a skill's data directory.",
            inputs: vec![
                skill_id_input("The skill ID."),
                FieldSchema {
                    name: "filename",
                    ty: TypeSchema::String,
                    comment: "Filename to read.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "content",
                ty: TypeSchema::String,
                comment: "File content.",
                required: true,
            }],
        },
        "data_write" => ControllerSchema {
            namespace: "skills",
            function: "data_write",
            description: "Write a file to a skill's data directory.",
            inputs: vec![
                skill_id_input("The skill ID."),
                FieldSchema {
                    name: "filename",
                    ty: TypeSchema::String,
                    comment: "Filename to write.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Content to write.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Write acknowledgment.",
                required: true,
            }],
        },
        "data_dir" => ControllerSchema {
            namespace: "skills",
            function: "data_dir",
            description: "Get the data directory path for a skill.",
            inputs: vec![skill_id_input("The skill ID.")],
            outputs: vec![FieldSchema {
                name: "path",
                ty: TypeSchema::String,
                comment: "Absolute path to the skill's data directory.",
                required: true,
            }],
        },
        "enable" => ControllerSchema {
            namespace: "skills",
            function: "enable",
            description: "Enable a skill (set preference and start it).",
            inputs: vec![skill_id_input("The skill ID to enable.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Enable acknowledgment.",
                required: true,
            }],
        },
        "disable" => ControllerSchema {
            namespace: "skills",
            function: "disable",
            description: "Disable a skill (set preference and stop it).",
            inputs: vec![skill_id_input("The skill ID to disable.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Disable acknowledgment.",
                required: true,
            }],
        },
        "is_enabled" => ControllerSchema {
            namespace: "skills",
            function: "is_enabled",
            description: "Check whether a skill is enabled in user preferences.",
            inputs: vec![skill_id_input("The skill ID to check.")],
            outputs: vec![FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "Whether the skill is enabled.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "skills",
            function: "unknown",
            description: "Unknown skills controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn skill_id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "skill_id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

// --- Skills registry handlers ---

#[derive(Deserialize)]
struct RegistryFetchParams {
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Deserialize)]
struct SearchParams {
    query: String,
    category: Option<String>,
}

#[derive(Deserialize)]
struct SkillIdParams {
    skill_id: String,
}

fn handle_skills_registry_fetch(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: RegistryFetchParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let config = config_rpc::load_config_with_timeout().await?;
        let registry =
            registry_ops::registry_fetch(&config.workspace_dir, p.force.unwrap_or(false)).await?;
        serde_json::to_value(registry).map_err(|e| e.to_string())
    })
}

fn handle_skills_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SearchParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let config = config_rpc::load_config_with_timeout().await?;
        let results =
            registry_ops::registry_search(&config.workspace_dir, &p.query, p.category.as_deref())
                .await?;
        serde_json::to_value(results).map_err(|e| e.to_string())
    })
}

fn handle_skills_install(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let config = config_rpc::load_config_with_timeout().await?;
        registry_ops::skill_install(&config.workspace_dir, &p.skill_id).await?;
        Ok(serde_json::json!({
            "success": true,
            "skill_id": p.skill_id
        }))
    })
}

fn handle_skills_uninstall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let config = config_rpc::load_config_with_timeout().await?;
        registry_ops::skill_uninstall(&config.workspace_dir, &p.skill_id).await?;
        Ok(serde_json::json!({
            "success": true,
            "skill_id": p.skill_id
        }))
    })
}

fn handle_skills_list_installed(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let installed = registry_ops::skills_list_installed(&config.workspace_dir).await?;
        serde_json::to_value(installed).map_err(|e| e.to_string())
    })
}

fn handle_skills_list_available(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let available = registry_ops::skills_list_available(&config.workspace_dir).await?;
        serde_json::to_value(available).map_err(|e| e.to_string())
    })
}

// --- Runtime handlers ---

#[derive(Deserialize)]
struct CallToolParams {
    skill_id: String,
    tool_name: String,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct SkillRpcParams {
    skill_id: String,
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

fn handle_skills_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let snapshot = engine.start_skill(&p.skill_id).await?;
        serde_json::to_value(&snapshot).map_err(|e| e.to_string())
    })
}

fn handle_skills_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine.stop_skill(&p.skill_id).await?;
        Ok(serde_json::json!({
            "success": true,
            "skill_id": p.skill_id
        }))
    })
}

fn handle_skills_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let snapshot = engine
            .get_skill_state(&p.skill_id)
            .ok_or_else(|| format!("Skill '{}' not found in runtime", p.skill_id))?;
        serde_json::to_value(&snapshot).map_err(|e| e.to_string())
    })
}

fn handle_skills_setup_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine
            .rpc(&p.skill_id, "setup/start", serde_json::json!({}))
            .await
    })
}

fn handle_skills_list_tools(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine
            .rpc(&p.skill_id, "tools/list", serde_json::json!({}))
            .await
    })
}

fn handle_skills_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine
            .rpc(&p.skill_id, "skill/tick", serde_json::json!({}))
            .await
    })
}

fn handle_skills_call_tool(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: CallToolParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let result = engine
            .call_tool(
                &p.skill_id,
                &p.tool_name,
                p.arguments.unwrap_or(serde_json::json!({})),
            )
            .await?;
        serde_json::to_value(&result).map_err(|e| e.to_string())
    })
}

fn handle_skills_rpc(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillRpcParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine
            .rpc(
                &p.skill_id,
                &p.method,
                p.params.unwrap_or(serde_json::json!({})),
            )
            .await
    })
}

fn handle_skills_discover(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let engine = require_engine()?;
        let manifests = engine.discover_skills().await?;
        serde_json::to_value(&manifests).map_err(|e| e.to_string())
    })
}

fn handle_skills_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let engine = require_engine()?;
        let skills = engine.list_skills();
        serde_json::to_value(&skills).map_err(|e| e.to_string())
    })
}

fn handle_skills_data_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let skill_id = params
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or("missing skill_id")?
            .to_string();
        let filename = params
            .get("filename")
            .and_then(|v| v.as_str())
            .ok_or("missing filename")?
            .to_string();
        let engine = require_engine()?;
        let content = engine.data_read(&skill_id, &filename)?;
        Ok(serde_json::json!({ "content": content }))
    })
}

fn handle_skills_data_write(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let skill_id = params
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or("missing skill_id")?
            .to_string();
        let filename = params
            .get("filename")
            .and_then(|v| v.as_str())
            .ok_or("missing filename")?
            .to_string();
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("missing content")?
            .to_string();
        let engine = require_engine()?;
        engine.data_write(&skill_id, &filename, &content)?;
        Ok(serde_json::json!({ "ok": true }))
    })
}

fn handle_skills_data_dir(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let path = engine.skill_data_dir(&p.skill_id);
        Ok(serde_json::json!({ "path": path.display().to_string() }))
    })
}

fn handle_skills_enable(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine.enable_skill(&p.skill_id).await?;
        Ok(serde_json::json!({ "ok": true }))
    })
}

fn handle_skills_disable(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine.disable_skill(&p.skill_id).await?;
        Ok(serde_json::json!({ "ok": true }))
    })
}

fn handle_skills_is_enabled(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let enabled = engine.is_skill_enabled(&p.skill_id);
        Ok(serde_json::json!({ "enabled": enabled }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_have_matching_handlers() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();
        assert_eq!(
            schemas.len(),
            controllers.len(),
            "schema count must match handler count"
        );
        for schema in &schemas {
            let method = format!("{}.{}", schema.namespace, schema.function);
            assert!(
                controllers
                    .iter()
                    .any(|c| c.schema.namespace == schema.namespace
                        && c.schema.function == schema.function),
                "no handler for schema '{method}'"
            );
        }
    }

    #[test]
    fn runtime_schemas_exist() {
        let schemas = all_controller_schemas();
        let expected = [
            "start",
            "stop",
            "status",
            "setup_start",
            "list_tools",
            "sync",
            "call_tool",
        ];
        for func in &expected {
            assert!(
                schemas
                    .iter()
                    .any(|s| s.namespace == "skills" && s.function == *func),
                "missing skills.{func} schema"
            );
        }
    }

    #[test]
    fn call_tool_schema_has_required_inputs() {
        let schema = skills_schema("call_tool");
        let required: Vec<&str> = schema
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert!(required.contains(&"skill_id"));
        assert!(required.contains(&"tool_name"));
        // arguments is optional
        let args = schema
            .inputs
            .iter()
            .find(|f| f.name == "arguments")
            .unwrap();
        assert!(!args.required);
    }

    #[tokio::test]
    async fn runtime_handlers_fail_without_engine() {
        // When no global engine is set, runtime handlers should return an error
        let cases = vec![
            ("start", serde_json::json!({"skill_id": "test"})),
            ("stop", serde_json::json!({"skill_id": "test"})),
            ("status", serde_json::json!({"skill_id": "test"})),
            ("setup_start", serde_json::json!({"skill_id": "test"})),
            ("list_tools", serde_json::json!({"skill_id": "test"})),
            ("sync", serde_json::json!({"skill_id": "test"})),
            (
                "call_tool",
                serde_json::json!({"skill_id": "test", "tool_name": "t"}),
            ),
        ];

        let controllers = all_registered_controllers();
        for (func, params) in cases {
            let controller = controllers
                .iter()
                .find(|c| c.schema.namespace == "skills" && c.schema.function == func)
                .unwrap_or_else(|| panic!("missing controller for skills.{func}"));

            let params_map: Map<String, Value> = params.as_object().unwrap().clone();
            let result = (controller.handler)(params_map).await;
            assert!(
                result.is_err(),
                "skills.{func} should fail without engine, got: {:?}",
                result
            );
            let err = result.unwrap_err();
            assert!(
                err.contains("runtime not initialized"),
                "skills.{func} error should mention runtime: {err}"
            );
        }
    }
}
