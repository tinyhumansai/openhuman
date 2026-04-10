use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentReplSessionStartParams {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentReplSessionControlParams {
    session_id: String,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("chat"),
        schemas("chat_simple"),
        schemas("repl_session_start"),
        schemas("repl_session_reset"),
        schemas("repl_session_end"),
        schemas("server_status"),
        schemas("list_definitions"),
        schemas("get_definition"),
        schemas("reload_definitions"),
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
            schema: schemas("repl_session_start"),
            handler: handle_repl_session_start,
        },
        RegisteredController {
            schema: schemas("repl_session_reset"),
            handler: handle_repl_session_reset,
        },
        RegisteredController {
            schema: schemas("repl_session_end"),
            handler: handle_repl_session_end,
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
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "repl_session_start" => ControllerSchema {
            namespace: "agent",
            function: "repl_session_start",
            description: "Create a persistent REPL agent session.",
            inputs: vec![
                optional_string("session_id", "Optional session id."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("result", "Session creation result.")],
        },
        "repl_session_reset" => ControllerSchema {
            namespace: "agent",
            function: "repl_session_reset",
            description: "Clear REPL session history.",
            inputs: vec![required_string("session_id", "REPL session id.")],
            outputs: vec![json_output("result", "Session reset result.")],
        },
        "repl_session_end" => ControllerSchema {
            namespace: "agent",
            function: "repl_session_end",
            description: "Terminate REPL session.",
            inputs: vec![required_string("session_id", "REPL session id.")],
            outputs: vec![json_output("result", "Session end result.")],
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
            description: "List all sub-agent definitions in the global registry \
                          (built-ins + custom TOML files under <workspace>/agents/).",
            inputs: vec![],
            outputs: vec![json_output("definitions", "Array of AgentDefinition.")],
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

fn handle_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_chat(
                &mut config,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_chat_simple(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_chat_simple(
                &config,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_repl_session_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentReplSessionStartParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_repl_session_start(
                &config,
                p.session_id,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_repl_session_reset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentReplSessionControlParams>(params)?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_repl_session_reset(p.session_id.trim()).await?,
        )
    })
}

fn handle_repl_session_end(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentReplSessionControlParams>(params)?;
        to_json(crate::openhuman::local_ai::rpc::agent_repl_session_end(p.session_id.trim()).await?)
    })
}

fn handle_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
}

fn handle_list_definitions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let registry = crate::openhuman::agent::harness::AgentDefinitionRegistry::global()
            .ok_or_else(|| "AgentDefinitionRegistry not initialised".to_string())?;
        let defs: Vec<&crate::openhuman::agent::harness::AgentDefinition> = registry.list();
        Ok(serde_json::json!({ "definitions": defs }))
    })
}

#[derive(Debug, Deserialize)]
struct GetDefinitionParams {
    id: String,
}

fn handle_get_definition(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<GetDefinitionParams>(params)?;
        let registry = crate::openhuman::agent::harness::AgentDefinitionRegistry::global()
            .ok_or_else(|| "AgentDefinitionRegistry not initialised".to_string())?;
        match registry.get(p.id.trim()) {
            Some(def) => Ok(serde_json::json!({ "definition": def })),
            None => Err(format!("definition '{}' not found", p.id)),
        }
    })
}

fn handle_reload_definitions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        // The global registry is OnceLock-backed so live reload is a
        // no-op in v1. Reply with a status payload that explains this
        // and tells the caller how to refresh.
        let already_loaded =
            crate::openhuman::agent::harness::AgentDefinitionRegistry::global().is_some();
        Ok(serde_json::json!({
            "status": "noop",
            "registry_initialised": already_loaded,
            "note": "Sub-agent definitions are loaded once at process startup. \
                     Restart the core process to pick up new TOML files under \
                     <workspace>/agents/.",
        }))
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
