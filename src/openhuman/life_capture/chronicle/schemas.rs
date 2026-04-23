//! Controller schemas + handler adapters for the chronicle domain.
//!
//! Controllers exposed:
//!   - `chronicle.list_recent`    — recent deduped/parsed focus events
//!   - `chronicle.get_watermark`  — last processed ts for a source
//!   - `chronicle.set_watermark`  — update the watermark for a source
//!
//! Handlers translate `Map<String, Value>` params into typed calls into
//! `rpc.rs`. The shared `PersonalIndex` handle is fetched from
//! `life_capture::runtime`, which core startup initialises.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::life_capture::chronicle::rpc;
use crate::openhuman::life_capture::runtime;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_recent"),
        schemas("get_watermark"),
        schemas("set_watermark"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_recent"),
            handler: handle_list_recent,
        },
        RegisteredController {
            schema: schemas("get_watermark"),
            handler: handle_get_watermark,
        },
        RegisteredController {
            schema: schemas("set_watermark"),
            handler: handle_set_watermark,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_recent" => ControllerSchema {
            namespace: "chronicle",
            function: "list_recent",
            description: "List recent chronicle focus events (S0 deduped + S1 parsed), newest first.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::U64,
                comment: "Maximum number of events to return. Defaults to 50, capped at 1000.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "events",
                ty: TypeSchema::Array(Box::new(TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "id",
                            ty: TypeSchema::I64,
                            comment: "Row id.",
                            required: true,
                        },
                        FieldSchema {
                            name: "ts_ms",
                            ty: TypeSchema::I64,
                            comment: "Event timestamp (unix milliseconds).",
                            required: true,
                        },
                        FieldSchema {
                            name: "focused_app",
                            ty: TypeSchema::String,
                            comment: "Bundle id or exe name of the focused app.",
                            required: true,
                        },
                        FieldSchema {
                            name: "focused_element",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "Accessibility role + label of the focused element, when available.",
                            required: false,
                        },
                        FieldSchema {
                            name: "visible_text",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "PII-redacted visible text of the focused element, when available.",
                            required: false,
                        },
                        FieldSchema {
                            name: "url",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "URL, populated only for browser-class apps.",
                            required: false,
                        },
                    ],
                })),
                comment: "Events ordered newest-first.",
                required: true,
            }],
        },
        "get_watermark" => ControllerSchema {
            namespace: "chronicle",
            function: "get_watermark",
            description: "Fetch the resumable watermark (last processed ts_ms) for a named source.",
            inputs: vec![FieldSchema {
                name: "source",
                ty: TypeSchema::String,
                comment: "Source identifier, e.g. 'focus' or 'calendar'. Required.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "last_ts_ms",
                ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                comment: "Last processed timestamp (unix ms), or null when the source has no watermark yet.",
                required: false,
            }],
        },
        "set_watermark" => ControllerSchema {
            namespace: "chronicle",
            function: "set_watermark",
            description: "Upsert the resumable watermark for a named source.",
            inputs: vec![
                FieldSchema {
                    name: "source",
                    ty: TypeSchema::String,
                    comment: "Source identifier. Required, non-empty.",
                    required: true,
                },
                FieldSchema {
                    name: "ts_ms",
                    ty: TypeSchema::I64,
                    comment: "Last processed timestamp (unix milliseconds).",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Always true on success.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "chronicle",
            function: "unknown",
            description: "Unknown chronicle function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Error message.",
                required: true,
            }],
        },
    }
}

fn handle_list_recent(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let idx = runtime::get_index().map_err(|e| e.to_string())?;
        let limit = read_optional_u64(&params, "limit")?.unwrap_or(50);
        let limit_u32 = limit.min(u32::MAX as u64) as u32;
        to_json(rpc::handle_list_recent(&idx, limit_u32).await?)
    })
}

fn handle_get_watermark(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let idx = runtime::get_index().map_err(|e| e.to_string())?;
        let source = read_required_string(&params, "source")?;
        to_json(rpc::handle_get_watermark(&idx, source).await?)
    })
}

fn handle_set_watermark(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let idx = runtime::get_index().map_err(|e| e.to_string())?;
        let source = read_required_string(&params, "source")?;
        let ts_ms = read_required_i64(&params, "ts_ms")?;
        to_json(rpc::handle_set_watermark(&idx, source, ts_ms).await?)
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

fn read_required_i64(params: &Map<String, Value>, key: &str) -> Result<i64, String> {
    match params.get(key) {
        Some(Value::Number(n)) => n
            .as_i64()
            .ok_or_else(|| format!("invalid '{key}': expected integer")),
        Some(other) => Err(format!(
            "invalid '{key}': expected integer, got {}",
            type_name(other)
        )),
        None => Err(format!("missing required param '{key}'")),
    }
}

fn read_optional_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected unsigned integer")),
        Some(other) => Err(format!(
            "invalid '{key}': expected unsigned integer, got {}",
            type_name(other)
        )),
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
    fn all_schemas_lists_three_functions() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(names, vec!["list_recent", "get_watermark", "set_watermark"]);
    }

    #[test]
    fn registered_handlers_match_schemas() {
        let regs = all_registered_controllers();
        assert_eq!(regs.len(), 3);
        let names: Vec<_> = regs.iter().map(|c| c.schema.function).collect();
        assert_eq!(names, vec!["list_recent", "get_watermark", "set_watermark"]);
    }

    #[test]
    fn set_watermark_requires_source_and_ts() {
        let s = schemas("set_watermark");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["source", "ts_ms"]);
    }

    #[test]
    fn unknown_function_returns_placeholder() {
        let s = schemas("does-not-exist");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }
}
