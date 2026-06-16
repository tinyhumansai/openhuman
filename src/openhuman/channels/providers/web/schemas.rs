use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::ops::{
    channel_web_cancel, channel_web_chat, channel_web_queue_clear, channel_web_queue_status,
};
use super::types::{ChatRequestMetadata, WebCancelParams, WebChatParams, WebQueueParams};

pub fn all_web_channel_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("chat"),
        schemas("cancel"),
        schemas("queue_status"),
        schemas("queue_clear"),
    ]
}

pub fn all_web_channel_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("chat"),
            handler: handle_chat,
        },
        RegisteredController {
            schema: schemas("cancel"),
            handler: handle_cancel,
        },
        RegisteredController {
            schema: schemas("queue_status"),
            handler: handle_queue_status,
        },
        RegisteredController {
            schema: schemas("queue_clear"),
            handler: handle_queue_clear,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "chat" => ControllerSchema {
            namespace: "channel",
            function: "web_chat",
            description: "Send a web channel message through the agent loop.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
                optional_string("profile_id", "Optional agent profile id."),
                optional_string(
                    "locale",
                    "Optional BCP-47 UI locale (e.g. 'ar', 'zh-CN'). Drives the \"reply in this language\" system-prompt directive.",
                ),
                optional_bool("speak_reply", "When true, the agent's final reply is spoken via TTS (for PTT and similar background voice flows)."),
                optional_string("source", "Origin of the message: \"ptt\" | \"dictation\" | \"type\" | other. Used for analytics + downstream metadata."),
                optional_u64("session_id", "Optional caller-provided correlation id (PTT session id)."),
                optional_string(
                    "queue_mode",
                    "Queue mode: 'interrupt' (default), 'steer', 'followup', or 'collect'.",
                ),
            ],
            outputs: vec![json_output("ack", "Acceptance payload.")],
        },
        "cancel" => ControllerSchema {
            namespace: "channel",
            function: "web_cancel",
            description: "Cancel in-flight web channel request for a thread.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
            ],
            outputs: vec![json_output("ack", "Cancellation payload.")],
        },
        "queue_status" => ControllerSchema {
            namespace: "channel",
            function: "web_queue_status",
            description: "Get the run queue status for a thread.",
            inputs: vec![required_string("thread_id", "Thread identifier.")],
            outputs: vec![json_output("status", "Queue status payload.")],
        },
        "queue_clear" => ControllerSchema {
            namespace: "channel",
            function: "web_queue_clear",
            description: "Clear the run queue for a thread.",
            inputs: vec![required_string("thread_id", "Thread identifier.")],
            outputs: vec![json_output("result", "Queue clear result.")],
        },
        _ => ControllerSchema {
            namespace: "channel",
            function: "unknown",
            description: "Unknown web channel controller function.",
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
        let p = deserialize_params::<WebChatParams>(params)?;
        to_json(
            channel_web_chat(
                &p.client_id,
                &p.thread_id,
                &p.message,
                p.model_override,
                p.temperature,
                p.profile_id,
                p.locale,
                p.queue_mode,
                ChatRequestMetadata {
                    speak_reply: p.speak_reply,
                    source: p.source,
                    session_id: p.session_id,
                },
            )
            .await?,
        )
    })
}

fn handle_queue_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebQueueParams>(params)?;
        to_json(channel_web_queue_status(&p.thread_id).await?)
    })
}

fn handle_queue_clear(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebQueueParams>(params)?;
        to_json(channel_web_queue_clear(&p.thread_id).await?)
    })
}

fn handle_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebCancelParams>(params)?;
        to_json(channel_web_cancel(&p.client_id, &p.thread_id).await?)
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

pub(crate) fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub(crate) fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

pub(crate) fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

pub(crate) fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

pub(crate) fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

pub(crate) fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
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
