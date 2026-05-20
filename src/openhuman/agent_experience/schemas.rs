use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent_experience::ops::{CaptureParams, DismissParams, RetrieveParams};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("capture"),
        schemas("retrieve"),
        schemas("list"),
        schemas("dismiss"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("capture"),
            handler: handle_capture,
        },
        RegisteredController {
            schema: schemas("retrieve"),
            handler: handle_retrieve,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("dismiss"),
            handler: handle_dismiss,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "capture" => ControllerSchema {
            namespace: "agent_experience",
            function: "capture",
            description:
                "Persist a redacted procedural operating experience for future agent turns.",
            inputs: vec![FieldSchema {
                name: "experience",
                ty: TypeSchema::Ref("AgentExperience"),
                comment: "Structured agent experience to upsert.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "experience",
                ty: TypeSchema::Ref("AgentExperience"),
                comment: "Stored agent experience.",
                required: true,
            }],
        },
        "retrieve" => ControllerSchema {
            namespace: "agent_experience",
            function: "retrieve",
            description: "Retrieve matching procedural operating experiences for a task.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Natural-language task query.",
                    required: true,
                },
                FieldSchema {
                    name: "tools",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tool names available or relevant to the task.",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tags used to bias retrieval.",
                    required: false,
                },
                FieldSchema {
                    name: "agent_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional agent definition id for same-agent boosts.",
                    required: false,
                },
                FieldSchema {
                    name: "entrypoint",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional turn entrypoint or event channel.",
                    required: false,
                },
                FieldSchema {
                    name: "max_hits",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of matching experiences to return. Defaults to 5.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "hits",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ExperienceHit"))),
                comment: "Ranked matching experiences.",
                required: true,
            }],
        },
        "list" => ControllerSchema {
            namespace: "agent_experience",
            function: "list",
            description: "List locally stored procedural operating experiences.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "experiences",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("AgentExperience"))),
                comment: "Stored agent experiences ordered by most recent update.",
                required: true,
            }],
        },
        "dismiss" => ControllerSchema {
            namespace: "agent_experience",
            function: "dismiss",
            description: "Mark an operating experience as dismissed so retrieval ignores it.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Experience id to dismiss.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "id",
                            ty: TypeSchema::String,
                            comment: "Requested experience id.",
                            required: true,
                        },
                        FieldSchema {
                            name: "dismissed",
                            ty: TypeSchema::Bool,
                            comment: "True when an existing experience was marked dismissed.",
                            required: true,
                        },
                    ],
                },
                comment: "Dismiss result.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "agent_experience",
            function: "unknown",
            description: "Unknown agent experience controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_capture(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = read_params::<CaptureParams>(params)?;
        to_json(crate::openhuman::agent_experience::ops::capture(params).await?)
    })
}

fn handle_retrieve(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = read_params::<RetrieveParams>(params)?;
        to_json(crate::openhuman::agent_experience::ops::retrieve(params).await?)
    })
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(crate::openhuman::agent_experience::ops::list().await?) })
}

fn handle_dismiss(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = read_params::<DismissParams>(params)?;
        to_json(crate::openhuman::agent_experience::ops::dismiss(params).await?)
    })
}

fn read_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| e.to_string())
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TypeSchema;
    use std::collections::BTreeSet;

    #[test]
    fn schemas_cover_capture_retrieve_list_and_dismiss() {
        let functions: BTreeSet<_> = all_controller_schemas()
            .into_iter()
            .map(|schema| schema.function)
            .collect();

        assert_eq!(
            functions,
            BTreeSet::from(["capture", "retrieve", "list", "dismiss"])
        );

        let registered: BTreeSet<_> = all_registered_controllers()
            .into_iter()
            .map(|controller| controller.schema.function)
            .collect();
        assert_eq!(registered, functions);
    }

    #[test]
    fn retrieve_schema_has_query_and_tools_inputs() {
        let schema = schemas("retrieve");
        assert_eq!(schema.namespace, "agent_experience");

        let query = schema
            .inputs
            .iter()
            .find(|input| input.name == "query")
            .expect("query input");
        assert_eq!(query.ty, TypeSchema::String);
        assert!(query.required);

        let tools = schema
            .inputs
            .iter()
            .find(|input| input.name == "tools")
            .expect("tools input");
        assert_eq!(tools.ty, TypeSchema::Array(Box::new(TypeSchema::String)));
        assert!(!tools.required);
    }
}
