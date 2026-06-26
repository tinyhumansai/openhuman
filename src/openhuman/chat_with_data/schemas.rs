//! Controller schemas for `chat_with_data` domain.
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use serde_json::{Map, Value};

type SB = fn() -> ControllerSchema;
type CH = fn(Map<String, Value>) -> ControllerFuture;
struct Def {
    function: &'static str,
    schema: SB,
    handler: CH,
}

const DEFS: &[Def] = &[
    Def {
        function: "register_dataset",
        schema: s_register,
        handler: h_register,
    },
    Def {
        function: "query",
        schema: s_query,
        handler: h_query,
    },
    Def {
        function: "generate_insight",
        schema: s_insight,
        handler: h_insight,
    },
    Def {
        function: "list_datasets",
        schema: s_list_ds,
        handler: h_list_ds,
    },
    Def {
        function: "list_insights",
        schema: s_list_ins,
        handler: h_list_ins,
    },
    Def {
        function: "get_dataset",
        schema: s_get,
        handler: h_get,
    },
    Def {
        function: "ingest_rows",
        schema: s_ingest,
        handler: h_ingest,
    },
    Def {
        function: "scan_anomalies",
        schema: s_scan,
        handler: h_scan,
    },
    Def {
        function: "delete_dataset",
        schema: s_delete,
        handler: h_delete,
    },
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    DEFS.iter().map(|d| (d.schema)()).collect()
}
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    DEFS.iter()
        .map(|d| RegisteredController {
            schema: (d.schema)(),
            handler: d.handler,
        })
        .collect()
}
pub fn schemas(function: &str) -> ControllerSchema {
    DEFS.iter()
        .find(|d| d.function == function)
        .map(|d| (d.schema)())
        .unwrap_or_else(s_unknown)
}

fn s_register() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "register_dataset",
        description: "Register a dataset for querying.",
        inputs: vec![
            FieldSchema {
                name: "name",
                ty: TypeSchema::String,
                comment: "Dataset name.",
                required: true,
            },
            FieldSchema {
                name: "source",
                ty: TypeSchema::String,
                comment: "csv|json|sqlite|api.",
                required: true,
            },
            FieldSchema {
                name: "columns",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Column names.",
                required: true,
            },
            FieldSchema {
                name: "row_count",
                ty: TypeSchema::U64,
                comment: "Row count.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "dataset_id",
                ty: TypeSchema::String,
                comment: "ID.",
                required: true,
            },
        ],
    }
}

fn s_query() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "query",
        description: "Ask a natural-language question over a dataset.",
        inputs: vec![
            FieldSchema {
                name: "dataset_id",
                ty: TypeSchema::String,
                comment: "Dataset ID.",
                required: true,
            },
            FieldSchema {
                name: "question",
                ty: TypeSchema::String,
                comment: "NL question.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "answer",
                ty: TypeSchema::String,
                comment: "Answer.",
                required: true,
            },
            FieldSchema {
                name: "confidence",
                ty: TypeSchema::F64,
                comment: "Confidence.",
                required: true,
            },
        ],
    }
}

fn s_insight() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "generate_insight",
        description: "Generate a proactive insight.",
        inputs: vec![FieldSchema {
            name: "dataset_id",
            ty: TypeSchema::String,
            comment: "Dataset ID.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "insight_id",
                ty: TypeSchema::String,
                comment: "Insight ID.",
                required: true,
            },
            FieldSchema {
                name: "title",
                ty: TypeSchema::String,
                comment: "Title.",
                required: true,
            },
        ],
    }
}

fn s_list_ds() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "list_datasets",
        description: "List registered datasets.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "datasets",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Datasets.",
                required: true,
            },
        ],
    }
}

fn s_list_ins() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "list_insights",
        description: "List generated insights.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "insights",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Insights.",
                required: true,
            },
        ],
    }
}

fn s_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "get_dataset",
        description: "Get dataset details.",
        inputs: vec![FieldSchema {
            name: "dataset_id",
            ty: TypeSchema::String,
            comment: "ID.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "dataset_id",
                ty: TypeSchema::String,
                comment: "ID.",
                required: true,
            },
            FieldSchema {
                name: "columns",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Columns.",
                required: true,
            },
        ],
    }
}

fn s_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "unknown",
        description: "Unknown.",
        inputs: vec![FieldSchema {
            name: "function",
            ty: TypeSchema::String,
            comment: "Requested.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Error.",
            required: true,
        }],
    }
}

fn h_register(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_register_dataset(p).await })
}
fn h_query(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_query(p).await })
}
fn h_insight(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_generate_insight(p).await })
}
fn h_list_ds(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_list_datasets(p).await })
}
fn h_list_ins(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_list_insights(p).await })
}
fn h_get(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_get_dataset(p).await })
}
fn h_ingest(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_ingest_rows(p).await })
}
fn h_scan(_p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_scan_anomalies(_p).await })
}
fn h_delete(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_delete_dataset(p).await })
}

fn s_ingest() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "ingest_rows",
        description: "Ingest rows into a dataset for in-memory querying.",
        inputs: vec![
            FieldSchema {
                name: "dataset_id",
                ty: TypeSchema::String,
                comment: "Dataset ID.",
                required: true,
            },
            FieldSchema {
                name: "rows",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Array of {col: value} objects.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "ingested",
                ty: TypeSchema::U64,
                comment: "Rows ingested.",
                required: true,
            },
        ],
    }
}

fn s_scan() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "scan_anomalies",
        description: "Proactively scan all datasets for anomalies.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "insights_found",
                ty: TypeSchema::U64,
                comment: "New insights.",
                required: true,
            },
        ],
    }
}

fn s_delete() -> ControllerSchema {
    ControllerSchema {
        namespace: "chat_with_data",
        function: "delete_dataset",
        description: "Remove a registered dataset.",
        inputs: vec![FieldSchema {
            name: "dataset_id",
            ty: TypeSchema::String,
            comment: "Dataset ID to delete.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "deleted",
                ty: TypeSchema::String,
                comment: "Deleted dataset ID.",
                required: true,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handlers_match() {
        assert_eq!(all_controller_schemas().len(), 9);
        assert_eq!(all_registered_controllers().len(), 9);
    }
    #[test]
    fn namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "chat_with_data");
        }
    }
    #[test]
    fn unknown() {
        assert_eq!(schemas("nope").function, "unknown");
    }
}
