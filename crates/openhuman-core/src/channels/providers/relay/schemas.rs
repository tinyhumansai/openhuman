//! The `channel.relay_inbound` controller (`openhuman.channel_relay_inbound`).
//!
//! It shares the `channel` namespace with `channel.web_chat`: both start a
//! turn on one of the caller's conversation threads.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use super::ops::channel_relay_inbound;
use super::params::RelayInboundParams;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_relay_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("relay_inbound")]
}

pub fn all_relay_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("relay_inbound"),
        handler: handle_relay_inbound,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "relay_inbound" => ControllerSchema {
            namespace: "channel",
            function: "relay_inbound",
            description: "Run a message relayed from a hosted chat platform (Telegram, iMessage, Discord, ...) on the caller's thread for that chat. Replies arrive as `channel_outbound` events on the caller's /events stream.",
            inputs: vec![
                field(
                    "channel",
                    TypeSchema::String,
                    "Platform name: lowercase [a-z0-9_-], starting with a letter (e.g. 'telegram').",
                    true,
                ),
                field(
                    "chat_id",
                    TypeSchema::String,
                    "Chat the message arrived in; replies go back to it.",
                    true,
                ),
                field(
                    "sender_id",
                    TypeSchema::String,
                    "Platform id of the sender.",
                    true,
                ),
                optional(
                    "sender_name",
                    TypeSchema::String,
                    "Sender display name, when the platform gives one.",
                ),
                field(
                    "message_id",
                    TypeSchema::String,
                    "Platform id of the message. A message already recorded is not run again.",
                    true,
                ),
                field("text", TypeSchema::String, "Message text.", true),
                optional(
                    "client_id",
                    TypeSchema::String,
                    "The /events client id replies are published to (default 'channel-relay').",
                ),
                optional(
                    "attachments",
                    TypeSchema::Json,
                    "Reserved; a non-empty list is refused.",
                ),
            ],
            outputs: vec![field(
                "ack",
                TypeSchema::Json,
                "{ accepted, thread_id, request_id, client_id }, or { accepted: false, duplicate: true, thread_id }.",
                true,
            )],
        },
        _ => ControllerSchema {
            namespace: "channel",
            function: "unknown",
            description: "Unknown channel relay controller function.",
            inputs: vec![],
            outputs: vec![field(
                "error",
                TypeSchema::String,
                "Lookup error details.",
                true,
            )],
        },
    }
}

fn handle_relay_inbound(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params: RelayInboundParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        channel_relay_inbound(&ctx, params)
            .await?
            .into_cli_compatible_json()
    })
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
}

fn optional(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    field(name, TypeSchema::Option(Box::new(ty)), comment, false)
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
