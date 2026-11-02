use serde::Deserialize;
use serde_json::{Map, Value};

use crate::agent_runs::agent_chat::params::AgentChatParams;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

#[derive(Debug, Deserialize)]
struct InferenceAgentChatParams {
    #[serde(flatten)]
    common: AgentChatParams,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    inference_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    inference_headers: Vec<(String, String)>,
    #[serde(default)]
    inference_provider_options: Option<Value>,
    #[serde(default)]
    inference_reasoning_effort: Option<String>,
}

pub(crate) fn schemas(function: &str) -> ControllerSchema {
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
                FieldSchema {
                    name: "inference_headers",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::Json)))),
                    comment:
                        "Custom header name/value pairs for inference_url only; never persisted.",
                    required: false,
                },
                FieldSchema {
                    name: "inference_provider_options",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment:
                        "Provider-specific defaults for inference_url; request options override them by key.",
                    required: false,
                },
                optional_string(
                    "inference_reasoning_effort",
                    "Reasoning effort for the model selected by inference_url.",
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

pub(crate) fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("agent_chat"),
            handler: handle_agent_chat,
        },
        RegisteredController {
            schema: schemas("agent_chat_simple"),
            handler: handle_agent_chat_simple,
        },
    ]
}

fn handle_agent_chat(
    ctx: crate::core::runtime::DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<InferenceAgentChatParams>(params)?;
        let mut config = ctx.load_config().await?;
        // Keep the resolved owner alive until the one-shot turn (including
        // spawned tools) has settled. The run context carries only a weak
        // reference so a persisted session cannot pin a tenant indefinitely.
        let core_lease = ctx.resolved();
        let target = match params.agent_id.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => super::AgentChatTarget::AgentId(id),
            _ => super::AgentChatTarget::Orchestrator,
        };
        let headless = ctx.is_headless_agent_chat();
        let reply = super::session_turn::agent_chat_reply_for_with_headless(
            &mut config,
            target,
            &params.common.message,
            params.common.model_override,
            params.common.temperature,
            params.common.thread_id,
            params.common.cwd,
            crate::config::schema::EphemeralRoute::from_params(
                params.inference_url,
                params.api_key,
            )
            .map(|route| {
                route
                    .with_headers(params.inference_headers)
                    .with_provider_options(params.inference_provider_options)
                    .with_reasoning_effort(params.inference_reasoning_effort)
            }),
            super::AgentChatEnv {
                host: crate::agent::turn_env::HostTurnEnv {
                    core: core_lease.as_ref().map(std::sync::Arc::downgrade),
                    ..Default::default()
                },
                ..Default::default()
            },
            headless,
        )
        .await?;
        drop(core_lease);
        reply.into_rpc_json()
    })
}

fn handle_agent_chat_simple(
    ctx: crate::core::runtime::DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<InferenceAgentChatParams>(params)?;
        let config = ctx.load_config().await?;
        crate::inference::host_runtime::ops::agent_chat_simple(
            &ctx,
            &config,
            &params.common.message,
            params.common.model_override,
            params.common.temperature,
            params.common.thread_id,
        )
        .await?
        .into_cli_compatible_json()
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
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

#[cfg(test)]
#[path = "controllers_tests.rs"]
mod tests;
