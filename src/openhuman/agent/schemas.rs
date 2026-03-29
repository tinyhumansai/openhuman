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
struct AgentReplSessionChatParams {
    session_id: String,
    message: String,
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
        schemas("repl_session_chat"),
        schemas("repl_session_reset"),
        schemas("repl_session_end"),
        schemas("server_status"),
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
            schema: schemas("repl_session_chat"),
            handler: handle_repl_session_chat,
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
        "repl_session_chat" => ControllerSchema {
            namespace: "agent",
            function: "repl_session_chat",
            description: "Send a message through REPL agent session.",
            inputs: vec![
                required_string("session_id", "REPL session id."),
                required_string("message", "User message."),
            ],
            outputs: vec![json_output("response", "Session chat response.")],
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

fn handle_repl_session_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentReplSessionChatParams>(params)?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_repl_session_chat(
                p.session_id.trim(),
                &p.message,
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
