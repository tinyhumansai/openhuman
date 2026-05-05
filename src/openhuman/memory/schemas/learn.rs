//! Schema and handler for the `memory.learn_all` RPC method.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc::{self, LearnAllParams};

use super::{parse_params, to_json};

pub(super) const FUNCTIONS: &[&str] = &["learn_all"];

pub(super) fn controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schema("learn_all").unwrap(),
        handler: handle_learn_all,
    }]
}

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    Some(match function {
        "learn_all" => ControllerSchema {
            namespace: "memory",
            function: "learn_all",
            description: "Run the tree summarizer over all memory namespaces (or a constrained subset). Processes namespaces sequentially; a failing namespace is recorded but does not abort the rest.",
            inputs: vec![FieldSchema {
                name: "namespaces",
                ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                comment: "Optional list of namespaces to constrain. Defaults to all namespaces.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "namespaces_processed",
                    ty: TypeSchema::U64,
                    comment: "Total number of namespaces processed.",
                    required: true,
                },
                FieldSchema {
                    name: "results",
                    ty: TypeSchema::Json,
                    comment: "Per-namespace outcomes: [{ namespace, status: 'ok'|'skipped'|'error', error? }].",
                    required: true,
                },
            ],
        },
        _ => return None,
    })
}

fn handle_learn_all(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<LearnAllParams>(params)?;
        to_json(rpc::memory_learn_all(payload).await?)
    })
}
