//! Schema and handler for the `memory.provider_status` RPC method
//! (`docs/specs/plan-memory.md` §5).

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc;

use super::to_json;

pub(super) const FUNCTIONS: &[&str] = &["provider_status"];

pub(super) fn controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schema("provider_status").unwrap(),
        handler: handle_provider_status,
    }]
}

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    Some(match function {
        "provider_status" => ControllerSchema {
            namespace: "memory",
            function: "provider_status",
            description: "Status of the bound memory driver: id, class, live health, contract version, advertised capability families, and any fallback.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "slot",
                    ty: TypeSchema::String,
                    comment: "Subsystem slot name; always \"memory\" here.",
                    required: true,
                },
                FieldSchema {
                    name: "driver",
                    ty: TypeSchema::String,
                    comment: "Bound driver id (tinycortex, supermemory, null). Empty when nothing could be bound.",
                    required: true,
                },
                FieldSchema {
                    name: "class",
                    ty: TypeSchema::String,
                    comment: "How the host bound the driver: embedded | external | null.",
                    required: true,
                },
                FieldSchema {
                    name: "health",
                    ty: TypeSchema::String,
                    comment: "Liveness as the driver reports it: ready | degraded | down.",
                    required: true,
                },
                FieldSchema {
                    name: "health_reason",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Operator-facing reason when degraded or down; null when ready.",
                    required: false,
                },
                FieldSchema {
                    name: "contract_version",
                    ty: TypeSchema::String,
                    comment: "Memory contract version this build speaks, as \"<major>.<minor>\".",
                    required: true,
                },
                FieldSchema {
                    name: "capabilities",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Advertised capability families as opaque snake_case strings.",
                    required: true,
                },
                FieldSchema {
                    name: "fell_back_from",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "The driver id that was asked for and refused, when this binding is a fallback.",
                    required: false,
                },
                FieldSchema {
                    name: "last_error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Last bind or call failure, operator-facing; null when clean.",
                    required: false,
                },
            ],
        },
        _ => return None,
    })
}

fn handle_provider_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::memory_provider_status().await) })
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
