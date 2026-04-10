//! JSON-RPC schemas and handlers for the OpenHuman Skills system.
//!
//! This module defines the interface between the frontend/RPC clients and the
//! skills registry and runtime: skill installation, and runtime control
//! (start, stop, tool calls, etc.).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

use super::qjs_engine::require_engine;
use super::registry_ops;
use super::types::{derive_connection_status, SkillSnapshot, SkillStatus};

/// Returns all controller schemas defined in this module.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
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
        skills_schema("data_stats"),
        skills_schema("enable"),
        skills_schema("disable"),
        skills_schema("is_enabled"),
        skills_schema("set_setup_complete"),
        skills_schema("get_all_snapshots"),
    ]
}

/// Returns all registered controllers (schema + handler) for the skills system.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
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
            schema: skills_schema("data_stats"),
            handler: handle_skills_data_stats,
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
        RegisteredController {
            schema: skills_schema("set_setup_complete"),
            handler: handle_skills_set_setup_complete,
        },
        RegisteredController {
            schema: skills_schema("get_all_snapshots"),
            handler: handle_skills_get_all_snapshots,
        },
    ]
}

// --- Skills registry schemas ---

/// Helper to create a schema for skills-related functions.
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
            description:
                "Send an arbitrary RPC method to a running skill (e.g. oauth/complete, skill/sync).",
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
            description:
                "Discover all available skill manifests from source and workspace directories.",
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
        "data_stats" => ControllerSchema {
            namespace: "skills",
            function: "data_stats",
            description: "Recursive file count and byte size for a skill's data directory.",
            inputs: vec![skill_id_input("The skill ID.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "exists, path, total_bytes, file_count.",
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
        "set_setup_complete" => ControllerSchema {
            namespace: "skills",
            function: "set_setup_complete",
            description: "Persist setup completion flag for a skill.",
            inputs: vec![
                skill_id_input("The skill ID."),
                FieldSchema {
                    name: "complete",
                    ty: TypeSchema::Bool,
                    comment: "Whether setup is complete.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Acknowledgment.",
                required: true,
            }],
        },
        "get_all_snapshots" => ControllerSchema {
            namespace: "skills",
            function: "get_all_snapshots",
            description:
                "Get all skill snapshots enriched with setup_complete and connection_status.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of enriched skill snapshots.",
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

/// Helper to create a standard `skill_id` input field schema.
fn skill_id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "skill_id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

// --- Skills registry handlers ---

/// Parameters for the `skills.registry_fetch` RPC method.
#[derive(Deserialize)]
struct RegistryFetchParams {
    /// If true, bypasses the disk cache and fetches a fresh copy from the remote registry.
    #[serde(default)]
    force: Option<bool>,
}

/// Parameters for the `skills.search` RPC method.
#[derive(Deserialize)]
struct SearchParams {
    /// The search query string (matches ID, name, or description).
    query: String,
    /// Optional category filter: "core" or "third_party".
    category: Option<String>,
}

/// Common parameters for RPC methods that take a `skill_id`.
#[derive(Deserialize)]
struct SkillIdParams {
    /// The unique identifier of the skill.
    skill_id: String,
}

/// RPC handler to fetch the remote skill registry.
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

/// RPC handler to search for available skills in the registry.
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

/// RPC handler to install a skill by ID.
fn handle_skills_install(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let config = config_rpc::load_config_with_timeout().await?;

        // Sync the engine's workspace_dir with the current config so that a
        // subsequent `skills_start` call finds the files we are about to write.
        // This is necessary when the user logged in *after* the core bootstrapped:
        // the engine was initialised with the pre-login (unscoped) workspace path,
        // but config.workspace_dir now points to the user-scoped directory.
        if let Ok(engine) = require_engine() {
            log::debug!(
                "[skills_install] syncing engine workspace_dir to {:?}",
                config.workspace_dir
            );
            engine.set_workspace_dir(config.workspace_dir.clone());
        }

        registry_ops::skill_install(&config.workspace_dir, &p.skill_id).await?;
        Ok(serde_json::json!({
            "success": true,
            "skill_id": p.skill_id
        }))
    })
}

/// RPC handler to uninstall a skill by ID.
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

/// RPC handler to list all locally installed skills.
fn handle_skills_list_installed(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let installed = registry_ops::skills_list_installed(&config.workspace_dir).await?;
        serde_json::to_value(installed).map_err(|e| e.to_string())
    })
}

/// RPC handler to list all available skills, enriched with installation status.
fn handle_skills_list_available(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let available = registry_ops::skills_list_available(&config.workspace_dir).await?;
        serde_json::to_value(available).map_err(|e| e.to_string())
    })
}

// --- Runtime handlers ---

/// Parameters for the `skills.call_tool` RPC method.
#[derive(Deserialize)]
struct CallToolParams {
    /// The ID of the skill that owns the tool.
    skill_id: String,
    /// The name of the tool to invoke.
    tool_name: String,
    /// Arguments to pass to the tool.
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

/// RPC handler to start (load and initialize) a skill.
fn handle_skills_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;

        // Refresh the engine's workspace_dir from config every time a skill is
        // started.  The engine's workspace_dir is set once at bootstrap using
        // the paths resolved at that moment.  If the user was not yet logged in
        // at startup the engine ends up with the unscoped legacy workspace path
        // (`~/.openhuman/workspace`), while `skills_install` writes to the
        // user-scoped path derived from `config.workspace_dir`.  Without this
        // sync `start_skill` cannot find the installed manifest and fails with
        // "Skill '…' not found (no manifest.json)".
        if let Ok(config) = config_rpc::load_config_with_timeout().await {
            log::debug!(
                "[skills_start] syncing engine workspace_dir to {:?}",
                config.workspace_dir
            );
            engine.set_workspace_dir(config.workspace_dir);
        }

        let snapshot = engine.start_skill(&p.skill_id).await?;
        serde_json::to_value(&snapshot).map_err(|e| e.to_string())
    })
}

/// RPC handler to stop a running skill.
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

/// RPC handler to get the current status and state of a skill.
fn handle_skills_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let snapshot = if let Some(snap) = engine.get_skill_state(&p.skill_id) {
            snap
        } else {
            // Not loaded in QuickJS (never started, still starting, or failed to start).
            // Still return persisted prefs — especially `setup_complete` after OAuth —
            // so the UI is not stuck waiting for a snapshot that would only exist once
            // the runtime has the skill registered.
            let setup_complete = engine.preferences().is_setup_complete(&p.skill_id);
            let status = SkillStatus::Pending;
            let state = HashMap::new();
            let connection_status = derive_connection_status(status, setup_complete, &state);
            SkillSnapshot {
                skill_id: p.skill_id.clone(),
                name: p.skill_id.clone(),
                status,
                tools: vec![],
                error: None,
                state,
                setup_complete,
                connection_status,
            }
        };
        serde_json::to_value(&snapshot).map_err(|e| e.to_string())
    })
}

/// RPC handler to initiate the setup flow for a skill.
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

/// RPC handler to list all tools exposed by a skill.
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

/// RPC handler to trigger a sync (tick) for a skill.
fn handle_skills_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        log::debug!("[skills] handle_skills_sync: skill_id={}", p.skill_id);
        let engine = require_engine()?;
        log::debug!(
            "[skills] handle_skills_sync: dispatching skill/sync to engine for '{}'",
            p.skill_id
        );
        engine
            .rpc(&p.skill_id, "skill/sync", serde_json::json!({}))
            .await
    })
}

/// RPC handler to call a specific tool on a running skill.
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

/// Parameters for the arbitrary `skills.rpc` method.
#[derive(Deserialize)]
struct SkillRpcParams {
    /// The target skill ID.
    skill_id: String,
    /// The internal RPC method name to call on the skill.
    method: String,
    /// Parameters for the internal RPC method.
    #[serde(default)]
    params: Option<serde_json::Value>,
}

/// RPC handler to send an arbitrary RPC method to a running skill.
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

/// RPC handler to discover available skill manifests from the filesystem.
fn handle_skills_discover(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let engine = require_engine()?;
        let manifests = engine.discover_skills().await?;
        serde_json::to_value(&manifests).map_err(|e| e.to_string())
    })
}

/// RPC handler to list all currently registered skill snapshots.
fn handle_skills_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let engine = require_engine()?;
        let skills = engine.list_skills();
        serde_json::to_value(&skills).map_err(|e| e.to_string())
    })
}

/// RPC handler to read a file from a skill's isolated data directory.
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

/// RPC handler to write a file to a skill's isolated data directory.
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

/// RPC handler to get the absolute path to a skill's data directory.
fn handle_skills_data_dir(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let path = engine.skill_data_dir(&p.skill_id);
        Ok(serde_json::json!({ "path": path.display().to_string() }))
    })
}

/// RPC handler to get storage statistics for a skill's data directory.
fn handle_skills_data_stats(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let stats = engine.skill_data_directory_stats(&p.skill_id);
        serde_json::to_value(&stats).map_err(|e| e.to_string())
    })
}

/// RPC handler to enable a skill in user preferences and start it.
fn handle_skills_enable(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine.enable_skill(&p.skill_id).await?;
        Ok(serde_json::json!({ "ok": true }))
    })
}

/// RPC handler to disable a skill in user preferences and stop it.
fn handle_skills_disable(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine.disable_skill(&p.skill_id).await?;
        Ok(serde_json::json!({ "ok": true }))
    })
}

/// RPC handler to check if a skill is currently enabled in user preferences.
fn handle_skills_is_enabled(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SkillIdParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        let enabled = engine.is_skill_enabled(&p.skill_id);
        Ok(serde_json::json!({ "enabled": enabled }))
    })
}

/// Parameters for the `skills.set_setup_complete` RPC method.
#[derive(Deserialize)]
struct SetSetupCompleteParams {
    /// The skill ID.
    skill_id: String,
    /// Whether the setup flow is considered complete.
    complete: bool,
}

/// RPC handler to persist the setup completion flag for a skill.
fn handle_skills_set_setup_complete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SetSetupCompleteParams =
            serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())?;
        let engine = require_engine()?;
        engine
            .preferences()
            .set_setup_complete(&p.skill_id, p.complete);
        Ok(serde_json::json!({
            "success": true,
            "skill_id": p.skill_id,
            "setup_complete": p.complete
        }))
    })
}

/// RPC handler to get all skill snapshots enriched with UI-specific metadata.
fn handle_skills_get_all_snapshots(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let engine = require_engine()?;
        let snapshots = engine.list_skills();
        serde_json::to_value(&snapshots).map_err(|e| e.to_string())
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
