use crate::core::runtime::DispatchCtx;
use crate::util::params::{json_output, optional_string, required_string, to_json};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use crate::agent_runs::agent_chat::params::AgentChatParams;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("chat"),
        schemas("chat_simple"),
        schemas("server_status"),
        schemas("list_definitions"),
        schemas("get_definition"),
        schemas("reload_definitions"),
        schemas("triage_evaluate"),
        schemas("registry_snapshot"),
        schemas("context_breakdown"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("chat"),
            handler: handle_chat,
        },
        RegisteredController {
            schema: schemas("chat_simple"),
            handler: handle_chat_simple,
        },
        RegisteredController {
            schema: schemas("server_status"),
            handler: handle_server_status,
        },
        RegisteredController {
            schema: schemas("list_definitions"),
            handler: handle_list_definitions,
        },
        RegisteredController {
            schema: schemas("get_definition"),
            handler: handle_get_definition,
        },
        RegisteredController {
            schema: schemas("reload_definitions"),
            handler: handle_reload_definitions,
        },
        RegisteredController {
            schema: schemas("triage_evaluate"),
            handler: crate::triage::rpc::handle_triage_evaluate,
        },
        RegisteredController {
            schema: schemas("registry_snapshot"),
            handler: crate::agent_registry::snapshot::handle_registry_snapshot,
        },
        RegisteredController {
            schema: schemas("context_breakdown"),
            handler: handle_context_breakdown,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "chat" => ControllerSchema {
            namespace: "agent",
            function: "chat",
            description: "Run one-shot agent chat with optional model overrides.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
                optional_string(
                    "thread_id",
                    "Optional backend thread id for cache grouping and inference logs.",
                ),
                optional_string(
                    "cwd",
                    "Optional working directory for this turn: the agent's file and shell \
                     tools are rooted here, so relative paths resolve inside it. Must be an \
                     existing directory. Omit (or pass empty) to use the configured action_dir.",
                ),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "chat_simple" => ControllerSchema {
            namespace: "agent",
            function: "chat_simple",
            description: "Run one-shot lightweight provider chat.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
                optional_string(
                    "thread_id",
                    "Optional backend thread id for cache grouping and inference logs.",
                ),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "server_status" => ControllerSchema {
            namespace: "agent",
            function: "server_status",
            description: "Return core runtime URL and status for agent calls.",
            inputs: vec![],
            outputs: vec![json_output("status", "Agent server status payload.")],
        },
        "list_definitions" => ControllerSchema {
            namespace: "agent",
            function: "list_definitions",
            description:
                "List safe display metadata for sub-agent definitions in the global registry.",
            inputs: vec![],
            outputs: vec![json_output(
                "definitions",
                "Array of safe AgentDefinitionDisplay payloads; prompt bodies are omitted.",
            )],
        },
        "get_definition" => ControllerSchema {
            namespace: "agent",
            function: "get_definition",
            description: "Fetch a single sub-agent definition by id.",
            inputs: vec![required_string("id", "Definition id (e.g. code_executor).")],
            outputs: vec![json_output("definition", "AgentDefinition payload.")],
        },
        "reload_definitions" => ControllerSchema {
            namespace: "agent",
            function: "reload_definitions",
            description: "Reload custom sub-agent definitions from disk. \
                          NOTE: only takes effect on next process restart in v1 \
                          since the global registry is OnceLock-backed.",
            inputs: vec![],
            outputs: vec![json_output("status", "Reload status payload.")],
        },
        "triage_evaluate" => crate::triage::rpc::triage_evaluate_schema(),
        "registry_snapshot" => crate::agent_registry::snapshot::registry_snapshot_schema(),
        "context_breakdown" => ControllerSchema {
            namespace: "agent",
            function: "context_breakdown",
            description: "Where an agent turn's fixed prompt budget goes: rendered system-prompt \
                          sections, advertised tool-schema bytes, and (with a thread_id) that \
                          thread's persisted history spend, as {label, bytes, est_tokens} rows \
                          the composer's context-usage indicator can render as a stacked bar. \
                          Expensive (rebuilds the agent and fetches live Composio connections); \
                          cached per agent id and invalidated only when config content changes.",
            inputs: vec![
                optional_string(
                    "agent_id",
                    "Agent whose prompt to measure. Defaults to 'orchestrator'.",
                ),
                optional_string(
                    "thread_id",
                    "When given, adds a 'history' section sized from this thread's persisted \
                     usage.",
                ),
            ],
            outputs: vec![json_output(
                "breakdown",
                "{agent_id, model, sections: [{label, bytes, est_tokens}], tools_bytes, \
                 total_est_tokens, context_window}.",
            )],
        },
        _ => ControllerSchema {
            namespace: "agent",
            function: "unknown",
            description: "Unknown agent controller function.",
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

fn handle_chat(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let mut config = ctx.load_config().await?;
        to_json(
            crate::agent_runs::agent_chat::agent_chat(
                &mut config,
                &p.message,
                p.model_override,
                p.temperature,
                p.thread_id,
                p.cwd,
                // `openhuman.agent_chat` describes a turn on the account's own
                // configured inference. A caller that wants this one turn
                // somewhere else says so through `inference_agent_chat`, which
                // takes the endpoint and bearer as parameters.
                None,
            )
            .await?,
        )
    })
}

fn handle_chat_simple(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(
            crate::inference::host_runtime::ops::agent_chat_simple(
                &ctx,
                &config,
                &p.message,
                p.model_override,
                p.temperature,
                p.thread_id,
            )
            .await?,
        )
    })
}

fn handle_server_status(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
}

fn handle_list_definitions(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let defs = crate::agent_registry::library::list_definition_metadata(&ctx).await?;
        Ok(serde_json::json!({ "definitions": defs }))
    })
}

#[derive(Debug, Deserialize)]
struct GetDefinitionParams {
    id: String,
}

fn handle_get_definition(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<GetDefinitionParams>(params)?;
        let registry = crate::agent_registry::host_definition::AgentDefinitionRegistry::current()
            .ok_or_else(|| "AgentDefinitionRegistry not initialised".to_string())?;
        match registry.get(p.id.trim()) {
            Some(def) => Ok(serde_json::json!({ "definition": def })),
            None => Err(format!("definition '{}' not found", p.id)),
        }
    })
}

fn handle_reload_definitions(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        // The global registry is OnceLock-backed so live reload is a
        // no-op in v1. Reply with a status payload that explains this
        // and tells the caller how to refresh.
        let already_loaded =
            crate::agent_registry::host_definition::AgentDefinitionRegistry::global().is_some();
        Ok(serde_json::json!({
            "status": "noop",
            "registry_initialised": already_loaded,
            "note": "Sub-agent definitions are loaded once at process startup. \
                     Restart the core process to pick up new TOML files under \
                     <workspace>/agents/.",
        }))
    })
}

fn handle_context_breakdown(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<crate::agent_runs::context_breakdown::ContextBreakdownParams>(
            params,
        )?;
        to_json(crate::agent_runs::context_breakdown::context_breakdown(&ctx, p).await?)
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
