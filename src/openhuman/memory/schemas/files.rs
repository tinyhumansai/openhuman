//! Schemas and handlers for file-based memory RPC methods.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc;
use crate::openhuman::memory::{
    ListMemoryFilesRequest, ReadMemoryFileRequest, WriteMemoryFileRequest,
};

use super::{parse_params, to_json};

pub(super) const FUNCTIONS: &[&str] = &["list_files", "read_file", "write_file"];

pub(super) fn controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("list_files").unwrap(),
            handler: handle_list_files,
        },
        RegisteredController {
            schema: schema("read_file").unwrap(),
            handler: handle_read_file,
        },
        RegisteredController {
            schema: schema("write_file").unwrap(),
            handler: handle_write_file,
        },
    ]
}

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    Some(match function {
        "list_files" => ControllerSchema {
            namespace: "memory",
            function: "list_files",
            description: "List files in a memory directory.",
            inputs: vec![FieldSchema {
                name: "relative_dir",
                ty: TypeSchema::String,
                comment: "Relative directory path under the workspace (default: \"memory\").",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with file listing.",
                required: true,
            }],
        },
        "read_file" => ControllerSchema {
            namespace: "memory",
            function: "read_file",
            description: "Read the contents of a memory file.",
            inputs: vec![FieldSchema {
                name: "relative_path",
                ty: TypeSchema::String,
                comment: "Relative path to the file under the workspace memory directory.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with file content.",
                required: true,
            }],
        },
        "write_file" => ControllerSchema {
            namespace: "memory",
            function: "write_file",
            description: "Write content to a memory file.",
            inputs: vec![
                FieldSchema {
                    name: "relative_path",
                    ty: TypeSchema::String,
                    comment: "Relative path to the file under the workspace memory directory.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Content to write to the file.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with write confirmation and bytes written.",
                required: true,
            }],
        },
        _ => return None,
    })
}

#[derive(serde::Deserialize)]
struct ListFilesParams {
    #[serde(default)]
    relative_dir: Option<String>,
}

fn handle_list_files(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // Reject invalid `relative_dir` types (e.g. `123`, `["x"]`) instead of
        // silently defaulting and masking client errors.
        let parsed: ListFilesParams = parse_params(params)?;
        // Empty string == the memory root itself (`<workspace>/memory`).
        let relative_dir = parsed.relative_dir.unwrap_or_default();
        let payload = ListMemoryFilesRequest { relative_dir };
        to_json(rpc::ai_list_memory_files(payload).await?)
    })
}

fn handle_read_file(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ReadMemoryFileRequest>(params)?;
        to_json(rpc::ai_read_memory_file(payload).await?)
    })
}

fn handle_write_file(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<WriteMemoryFileRequest>(params)?;
        to_json(rpc::ai_write_memory_file(payload).await?)
    })
}
