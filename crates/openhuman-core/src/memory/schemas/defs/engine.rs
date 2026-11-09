use super::common::{opt, out, req};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "engines_list" => Some(ControllerSchema {
            namespace: "memory",
            function: "engines_list",
            description: "List the memory engines this build offers and the active one.",
            inputs: vec![],
            outputs: out("{engines: EngineDescriptor[], active: string|null}"),
        }),
        "engine_get" => Some(ControllerSchema {
            namespace: "memory",
            function: "engine_get",
            description: "The configured memory engine, its credential and health.",
            inputs: vec![],
            outputs: out("{engine, endpoint?, has_key, status, reason?, fetch_modes}"),
        }),
        "engine_set" => Some(ControllerSchema {
            namespace: "memory",
            function: "engine_set",
            description: "Select a memory engine, optionally setting its endpoint and API key.",
            inputs: vec![
                req(
                    "engine",
                    TypeSchema::String,
                    "Engine id: tinyhumans or cortexdb.",
                ),
                opt(
                    "endpoint",
                    TypeSchema::String,
                    "Endpoint URL; empty clears it.",
                ),
                opt(
                    "api_key",
                    TypeSchema::String,
                    "API key (cortexdb); empty removes it. Stored in the credential store, never in config.",
                ),
            ],
            outputs: out("Same as memory_engine_get."),
        }),
        _ => None,
    }
}
