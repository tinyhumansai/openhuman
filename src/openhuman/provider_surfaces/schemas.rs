//! Controller registry for `provider_surfaces`.
//!
//! The first cut exposes normalized provider event ingestion plus a queue
//! listing endpoint suitable for local-first assistive UI surfaces.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::EmptyRequest;

use super::ops;
use super::types::ProviderEvent;

pub fn all_provider_surfaces_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("ingest_event"), schemas("list_queue")]
}

pub fn all_provider_surfaces_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("ingest_event"),
            handler: handle_ingest_event,
        },
        RegisteredController {
            schema: schemas("list_queue"),
            handler: handle_list_queue,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "ingest_event" => ControllerSchema {
            namespace: "provider_surfaces",
            function: "ingest_event",
            description: "Ingest a normalized provider event into the local respond queue.",
            inputs: vec![
                field("provider", TypeSchema::String, "Provider slug (e.g. linkedin, gmail)."),
                field("account_id", TypeSchema::String, "Provider account identifier."),
                field("event_kind", TypeSchema::String, "Normalized event kind (e.g. message, mention)."),
                field("entity_id", TypeSchema::String, "Stable provider entity identifier."),
                optional("thread_id", TypeSchema::String, "Optional thread or conversation id."),
                optional("title", TypeSchema::String, "Short human-readable title."),
                optional("snippet", TypeSchema::String, "Preview snippet for queue rendering."),
                optional("sender_name", TypeSchema::String, "Human-readable sender name."),
                optional("sender_handle", TypeSchema::String, "Stable sender handle."),
                field("timestamp", TypeSchema::String, "RFC3339 timestamp for the event."),
                optional("deep_link", TypeSchema::String, "Provider deep link used to open the source surface."),
                FieldSchema {
                    name: "requires_attention",
                    // ProviderEvent::requires_attention is #[serde(default)] so
                    // the deserializer accepts absence. Mark required: false here
                    // so the registry's validate_params agrees with the struct.
                    ty: TypeSchema::Bool,
                    comment: "Whether the event should enter the respond queue as actionable. Defaults to false when omitted.",
                    required: false,
                },
                FieldSchema {
                    name: "raw_payload",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional provider-specific raw payload for future debugging and enrichment.",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Envelope containing the upserted queue item.")],
        },
        "list_queue" => ControllerSchema {
            namespace: "provider_surfaces",
            function: "list_queue",
            description: "List the local respond queue derived from provider events.",
            inputs: vec![],
            outputs: vec![json_output("result", "Envelope containing queue items and count.")],
        },
        _ => ControllerSchema {
            namespace: "provider_surfaces",
            function: "unknown",
            description: "Unknown provider_surfaces controller.",
            inputs: vec![],
            outputs: vec![field("error", TypeSchema::String, "Lookup error details.")],
        },
    }
}

fn handle_ingest_event(params: Map<String, Value>) -> crate::core::all::ControllerFuture {
    Box::pin(async move {
        let payload: ProviderEvent = parse_params(params)?;
        ops::ingest_event(payload).await?.into_cli_compatible_json()
    })
}

fn handle_list_queue(params: Map<String, Value>) -> crate::core::all::ControllerFuture {
    Box::pin(async move {
        let payload: EmptyRequest = parse_params(params)?;
        ops::list_queue(payload).await?.into_cli_compatible_json()
    })
}

fn parse_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

fn optional(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(ty)),
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
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_two() {
        assert_eq!(all_provider_surfaces_controller_schemas().len(), 2);
    }

    #[test]
    fn all_controllers_returns_two() {
        assert_eq!(all_provider_surfaces_registered_controllers().len(), 2);
    }

    #[test]
    fn list_queue_schema_has_no_inputs() {
        let schema = schemas("list_queue");
        assert!(schema.inputs.is_empty());
        assert_eq!(schema.namespace, "provider_surfaces");
    }
}
