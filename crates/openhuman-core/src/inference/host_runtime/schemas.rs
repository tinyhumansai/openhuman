use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
struct LocalAiTestConnectionParams {
    url: String,
}

use crate::config::rpc as config_rpc;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

/// Params for `inference.agent_chat` and `inference.agent_chat_simple`.
///
/// A near-copy of the params in [`crate::agent::schemas`], which
/// backs the `agent.chat` / `agent.chat_simple` namespace over the same ops.
/// The two diverge in one field: only this surface accepts a per-call
/// `inference_url` + `api_key`, because `agent.chat` describes a turn on the
/// account's own configured inference.
#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
    /// Which agent definition to run the turn as. Absent runs the
    /// orchestrator, as before. Resolved through the definition registry and
    /// then `config.agent_registry.entries`; an unknown id is an error.
    #[serde(default)]
    agent_id: Option<String>,
    /// Optional per-turn working directory for the agent's filesystem / shell
    /// tools. Absent or empty keeps the configured `action_dir`. Ignored by the
    /// `*_simple` variant, which runs a bare provider call with no tools.
    cwd: Option<String>,
    /// OpenAI-compatible endpoint this one call should run against. Paired with
    /// `api_key`; either alone is ignored. See
    /// [`ephemeral_route`](crate::config::schema::ephemeral_route).
    #[serde(default)]
    inference_url: Option<String>,
    /// Bearer for `inference_url`. Never persisted and never handed to any
    /// other provider.
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeParams {
    audio_path: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeBytesParams {
    audio_bytes: Vec<u8>,
    extension: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiTtsParams {
    text: String,
    output_path: Option<String>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("agent_chat"),
        schemas("agent_chat_simple"),
        schemas("transcribe"),
        schemas("transcribe_bytes"),
        schemas("tts"),
        schemas("test_connection"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("agent_chat"),
            handler: handle_agent_chat,
        },
        RegisteredController {
            schema: schemas("agent_chat_simple"),
            handler: handle_agent_chat_simple,
        },
        RegisteredController {
            schema: schemas("transcribe"),
            handler: handle_local_ai_transcribe,
        },
        RegisteredController {
            schema: schemas("transcribe_bytes"),
            handler: handle_local_ai_transcribe_bytes,
        },
        RegisteredController {
            schema: schemas("tts"),
            handler: handle_local_ai_tts,
        },
        RegisteredController {
            schema: schemas("test_connection"),
            handler: handle_local_ai_test_connection,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "agent_chat" => ControllerSchema {
            namespace: "inference",
            function: "agent_chat",
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
                    "agent_id",
                    "Optional agent definition id to run the turn as. Omit for the \
                     orchestrator.",
                ),
                optional_string(
                    "cwd",
                    "Optional working directory for this turn: the agent's file and shell \
                     tools are rooted here, so relative paths resolve inside it. Must be an \
                     existing directory. Omit (or pass empty) to use the configured action_dir.",
                ),
                optional_string(
                    "inference_url",
                    "Optional OpenAI-compatible endpoint for this call only. \
                     Requires api_key; never persisted.",
                ),
                optional_string(
                    "api_key",
                    "Bearer for inference_url. Scoped to this call and to that \
                     endpoint alone.",
                ),
            ],
            outputs: vec![json_output(
                "response",
                "Agent response payload: `result` (the reply text), `logs`, `hit_cap` (true when \
                 the turn stopped at its tool-iteration cap instead of finishing) and, only when \
                 hit_cap is true, `checkpoint` (the resumable checkpoint text, equal to `result`).",
            )],
        },
        "agent_chat_simple" => ControllerSchema {
            namespace: "inference",
            function: "agent_chat_simple",
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
        "transcribe" => ControllerSchema {
            namespace: "inference",
            function: "transcribe",
            description: "Transcribe audio from file path.",
            inputs: vec![required_string("audio_path", "Input audio path.")],
            outputs: vec![json_output("speech", "Transcription payload.")],
        },
        "transcribe_bytes" => ControllerSchema {
            namespace: "inference",
            function: "transcribe_bytes",
            description: "Transcribe audio from raw bytes.",
            inputs: vec![
                FieldSchema {
                    name: "audio_bytes",
                    ty: TypeSchema::Bytes,
                    comment: "Raw audio bytes.",
                    required: true,
                },
                optional_string("extension", "Optional audio extension."),
            ],
            outputs: vec![json_output("speech", "Transcription payload.")],
        },
        "tts" => ControllerSchema {
            namespace: "inference",
            function: "tts",
            description: "Synthesize speech from text.",
            inputs: vec![
                required_string("text", "Input text."),
                optional_string("output_path", "Optional output path."),
            ],
            outputs: vec![json_output("tts", "TTS result payload.")],
        },
        "test_connection" => ControllerSchema {
            namespace: "inference",
            function: "test_connection",
            description: "Test connectivity to an Ollama server URL. Returns reachable status and model count.",
            inputs: vec![required_string("url", "Ollama server URL to test.")],
            outputs: vec![json_output("result", "Connection test result.")],
        },
        _ => ControllerSchema {
            namespace: "inference",
            function: "unknown",
            description: "Unknown local inference controller function.",
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

fn handle_agent_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;
        let target = match p.agent_id.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => {
                crate::inference::host_runtime::ops::AgentChatTarget::AgentId(id)
            }
            _ => crate::inference::host_runtime::ops::AgentChatTarget::Orchestrator,
        };
        crate::inference::host_runtime::ops::agent_chat_reply_for(
            &mut config,
            target,
            &p.message,
            p.model_override,
            p.temperature,
            p.thread_id,
            p.cwd,
            crate::config::schema::EphemeralRoute::from_params(p.inference_url, p.api_key),
        )
        .await?
        .into_rpc_json()
    })
}

fn handle_agent_chat_simple(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::inference::host_runtime::ops::agent_chat_simple(
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

fn handle_local_ai_transcribe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::inference::host_runtime::ops::local_ai_transcribe(&config, p.audio_path.trim())
                .await?,
        )
    })
}

fn handle_local_ai_transcribe_bytes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeBytesParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::inference::host_runtime::ops::local_ai_transcribe_bytes(
                &config,
                &p.audio_bytes,
                p.extension,
            )
            .await?,
        )
    })
}

fn handle_local_ai_tts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTtsParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::inference::host_runtime::ops::local_ai_tts(
                &config,
                &p.text,
                p.output_path.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_local_ai_test_connection(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTestConnectionParams>(params)?;
        let result = tinyinference_local::service::test_ollama_connection(&p.url).await?;
        serde_json::to_value(result).map_err(|e| format!("serialize test_connection result: {e}"))
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

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
