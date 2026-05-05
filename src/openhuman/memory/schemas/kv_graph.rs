//! Schemas and handlers for key-value and knowledge-graph RPC methods.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc::{
    self, GraphQueryParams, GraphUpsertParams, KvGetDeleteParams, KvSetParams, NamespaceOnlyParams,
};

use super::{parse_params, to_json};

pub(super) const FUNCTIONS: &[&str] = &[
    "kv_set",
    "kv_get",
    "kv_delete",
    "kv_list_namespace",
    "graph_upsert",
    "graph_query",
];

pub(super) fn controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("kv_set").unwrap(),
            handler: handle_kv_set,
        },
        RegisteredController {
            schema: schema("kv_get").unwrap(),
            handler: handle_kv_get,
        },
        RegisteredController {
            schema: schema("kv_delete").unwrap(),
            handler: handle_kv_delete,
        },
        RegisteredController {
            schema: schema("kv_list_namespace").unwrap(),
            handler: handle_kv_list_namespace,
        },
        RegisteredController {
            schema: schema("graph_upsert").unwrap(),
            handler: handle_graph_upsert,
        },
        RegisteredController {
            schema: schema("graph_query").unwrap(),
            handler: handle_graph_query,
        },
    ]
}

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    Some(match function {
        "kv_set" => ControllerSchema {
            namespace: "memory",
            function: "kv_set",
            description: "Set a key-value pair in the memory store.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Key to set.",
                    required: true,
                },
                FieldSchema {
                    name: "value",
                    ty: TypeSchema::Json,
                    comment: "JSON value to store.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when the value was stored.",
                required: true,
            }],
        },
        "kv_get" => ControllerSchema {
            namespace: "memory",
            function: "kv_get",
            description: "Get a value by key from the memory store.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Key to retrieve.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Stored value or null if not found.",
                required: true,
            }],
        },
        "kv_delete" => ControllerSchema {
            namespace: "memory",
            function: "kv_delete",
            description: "Delete a key-value pair from the memory store.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "key",
                    ty: TypeSchema::String,
                    comment: "Key to delete.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when the key was deleted.",
                required: true,
            }],
        },
        "kv_list_namespace" => ControllerSchema {
            namespace: "memory",
            function: "kv_list_namespace",
            description: "List all key-value entries in a namespace.",
            inputs: vec![FieldSchema {
                name: "namespace",
                ty: TypeSchema::String,
                comment: "Namespace to list.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of key-value entries.",
                required: true,
            }],
        },
        "graph_upsert" => ControllerSchema {
            namespace: "memory",
            function: "graph_upsert",
            description: "Upsert a relation triple in the knowledge graph.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "subject",
                    ty: TypeSchema::String,
                    comment: "Subject entity of the relation.",
                    required: true,
                },
                FieldSchema {
                    name: "predicate",
                    ty: TypeSchema::String,
                    comment: "Relation predicate.",
                    required: true,
                },
                FieldSchema {
                    name: "object",
                    ty: TypeSchema::String,
                    comment: "Object entity of the relation.",
                    required: true,
                },
                FieldSchema {
                    name: "attrs",
                    ty: TypeSchema::Json,
                    comment: "Extra attributes on the relation (default: {}).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when the relation was upserted.",
                required: true,
            }],
        },
        "graph_query" => ControllerSchema {
            namespace: "memory",
            function: "graph_query",
            description: "Query relations from the knowledge graph.",
            inputs: vec![
                FieldSchema {
                    name: "namespace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional namespace scope.",
                    required: false,
                },
                FieldSchema {
                    name: "subject",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Filter by subject entity.",
                    required: false,
                },
                FieldSchema {
                    name: "predicate",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Filter by relation predicate.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of matching relation records.",
                required: true,
            }],
        },
        _ => return None,
    })
}

fn handle_kv_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<KvSetParams>(params)?;
        to_json(rpc::kv_set(payload).await?)
    })
}

fn handle_kv_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<KvGetDeleteParams>(params)?;
        to_json(rpc::kv_get(payload).await?)
    })
}

fn handle_kv_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<KvGetDeleteParams>(params)?;
        to_json(rpc::kv_delete(payload).await?)
    })
}

fn handle_kv_list_namespace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<NamespaceOnlyParams>(params)?;
        to_json(rpc::kv_list_namespace(payload).await?)
    })
}

fn handle_graph_upsert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<GraphUpsertParams>(params)?;
        to_json(rpc::graph_upsert(payload).await?)
    })
}

fn handle_graph_query(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<GraphQueryParams>(params)?;
        to_json(rpc::graph_query(payload).await?)
    })
}
