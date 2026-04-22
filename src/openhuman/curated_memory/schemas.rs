//! Controller schemas + handler adapters for curated memory.
//!
//! Exposes four controllers under the `memory_curated` namespace (the
//! `memory` namespace is already owned by the long-term memory subsystem):
//!   - memory_curated.read    — read a curated file ("memory" | "user")
//!   - memory_curated.add     — append an entry, char-bounded
//!   - memory_curated.replace — substring replace
//!   - memory_curated.remove  — drop entries matching a needle

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::curated_memory::{rpc, runtime};
use crate::rpc::RpcOutcome;

const NS: &str = "memory_curated";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("read"),
        schemas("add"),
        schemas("replace"),
        schemas("remove"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController { schema: schemas("read"), handler: handle_read },
        RegisteredController { schema: schemas("add"), handler: handle_add },
        RegisteredController { schema: schemas("replace"), handler: handle_replace },
        RegisteredController { schema: schemas("remove"), handler: handle_remove },
    ]
}

fn file_input() -> FieldSchema {
    FieldSchema {
        name: "file",
        ty: TypeSchema::Enum { variants: vec!["memory", "user"] },
        comment: "Which curated file to operate on: 'memory' (agent notes) or 'user' (user notes).",
        required: true,
    }
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "read" => ControllerSchema {
            namespace: NS,
            function: "read",
            description: "Read the full body of a curated memory file.",
            inputs: vec![file_input()],
            outputs: vec![
                FieldSchema { name: "file", ty: TypeSchema::String, comment: "Echoed file selector.", required: true },
                FieldSchema { name: "body", ty: TypeSchema::String, comment: "Current file contents (may be empty).", required: true },
            ],
        },
        "add" => ControllerSchema {
            namespace: NS,
            function: "add",
            description: "Append an entry to a curated memory file. Rejected if the resulting file would exceed its char limit.",
            inputs: vec![
                file_input(),
                FieldSchema { name: "entry", ty: TypeSchema::String, comment: "The entry text to append.", required: true },
            ],
            outputs: vec![
                FieldSchema { name: "file", ty: TypeSchema::String, comment: "Echoed file selector.", required: true },
                FieldSchema { name: "ok", ty: TypeSchema::Bool, comment: "True on success.", required: true },
            ],
        },
        "replace" => ControllerSchema {
            namespace: NS,
            function: "replace",
            description: "Substring replace inside a curated memory file. Rejected if the result would exceed the char limit.",
            inputs: vec![
                file_input(),
                FieldSchema { name: "needle", ty: TypeSchema::String, comment: "Substring to find.", required: true },
                FieldSchema { name: "replacement", ty: TypeSchema::String, comment: "Replacement text.", required: true },
            ],
            outputs: vec![
                FieldSchema { name: "file", ty: TypeSchema::String, comment: "Echoed file selector.", required: true },
                FieldSchema { name: "ok", ty: TypeSchema::Bool, comment: "True on success.", required: true },
            ],
        },
        "remove" => ControllerSchema {
            namespace: NS,
            function: "remove",
            description: "Drop any entries containing the given needle from a curated memory file.",
            inputs: vec![
                file_input(),
                FieldSchema { name: "needle", ty: TypeSchema::String, comment: "Substring; entries containing it are removed.", required: true },
            ],
            outputs: vec![
                FieldSchema { name: "file", ty: TypeSchema::String, comment: "Echoed file selector.", required: true },
                FieldSchema { name: "ok", ty: TypeSchema::Bool, comment: "True on success.", required: true },
            ],
        },
        _ => ControllerSchema {
            namespace: NS,
            function: "unknown",
            description: "Unknown curated memory function.",
            inputs: vec![],
            outputs: vec![FieldSchema { name: "error", ty: TypeSchema::String, comment: "Error message.", required: true }],
        },
    }
}

fn handle_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rt = runtime::get().map_err(|e| e.to_string())?;
        let file = read_required_string(&params, "file")?;
        to_json(rpc::handle_read(&rt, file).await?)
    })
}

fn handle_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rt = runtime::get().map_err(|e| e.to_string())?;
        let file = read_required_string(&params, "file")?;
        let entry = read_required_string(&params, "entry")?;
        to_json(rpc::handle_add(&rt, file, entry).await?)
    })
}

fn handle_replace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rt = runtime::get().map_err(|e| e.to_string())?;
        let file = read_required_string(&params, "file")?;
        let needle = read_required_string(&params, "needle")?;
        let replacement = read_required_string(&params, "replacement")?;
        to_json(rpc::handle_replace(&rt, file, needle, replacement).await?)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rt = runtime::get().map_err(|e| e.to_string())?;
        let file = read_required_string(&params, "file")?;
        let needle = read_required_string(&params, "needle")?;
        to_json(rpc::handle_remove(&rt, file, needle).await?)
    })
}

fn read_required_string(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    match params.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
            type_name(other)
        )),
        None => Err(format!("missing required param '{key}'")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_cover_all_four_functions() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(names, vec!["read", "add", "replace", "remove"]);
    }

    #[test]
    fn schemas_unknown_returns_placeholder() {
        let s = schemas("nope");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn file_input_is_enum_with_two_variants() {
        let s = schemas("read");
        let f = &s.inputs[0];
        assert_eq!(f.name, "file");
        match &f.ty {
            TypeSchema::Enum { variants } => assert_eq!(variants, &vec!["memory", "user"]),
            other => panic!("expected enum, got {other:?}"),
        }
    }

    #[test]
    fn registered_controllers_align_with_schemas() {
        let regs = all_registered_controllers();
        assert_eq!(regs.len(), 4);
        let names: Vec<_> = regs.iter().map(|c| c.schema.function).collect();
        assert_eq!(names, vec!["read", "add", "replace", "remove"]);
    }
}
