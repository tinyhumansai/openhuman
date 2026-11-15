//! Shared controller-schema helpers: JSON-RPC param decoding, outcome
//! conversion and the common `FieldSchema` builders.
//!
//! Every domain's `schemas.rs` decodes its controller params out of a
//! `serde_json::Map<String, Value>` by hand. [`read_required`] and
//! [`read_optional`] are the two-line core of that boilerplate — reused here
//! instead of being hand-copied per domain, so the "missing required param"
//! / "invalid '<key>': <serde error>" message shapes stay one contract. The
//! rest of this file does the same for the other helpers schema files used to
//! copy: [`to_json`], [`json_output`], [`type_name`] and the rest.
//!
//! The one `crate::` dependency is `crate::core`, the kernel that defines
//! [`Outcome`] and [`FieldSchema`]. No domain is reached from here.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::core::{FieldSchema, Outcome, TypeSchema};

// ── Param decoding ───────────────────────────────────────────────────────────

/// Deserialize a required param `key`, failing with a `String` error when
/// it's absent or doesn't match `T`'s shape.
pub fn read_required<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

/// Deserialize an optional param `key`. Absent or `null` yields `Ok(None)`;
/// present-but-wrong-shaped yields the same `"invalid '<key>': ..."` error as
/// [`read_required`].
pub fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

/// Read an optional `u32` param `key`. Absent or `null` yields `Ok(None)`.
/// Unlike `read_optional::<u32>`, the error names the JSON type it got
/// (`"invalid '<key>': expected number, got <type>"`), and a negative,
/// fractional or too-large number fails with `"invalid '<key>': expected u32"`.
pub fn read_optional_u32(params: &Map<String, Value>, key: &str) -> Result<Option<u32>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected u32")),
        Some(other) => Err(format!(
            "invalid '{key}': expected number, got {}",
            type_name(other)
        )),
    }
}

/// The JSON type name of `value` (`"null"`, `"bool"`, `"number"`, `"string"`,
/// `"array"`, `"object"`), for `"expected <x>, got <type>"` errors.
pub fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A trimmed, non-empty string from agent-tool `args[key]`. Absent,
/// non-string and blank values all yield `None`.
pub fn opt_trimmed_str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ── Outcome conversion ───────────────────────────────────────────────────────

/// Convert a controller [`Outcome`] into its RPC JSON through
/// [`Outcome::into_cli_compatible_json`]: the bare value, or
/// `{ "result", "logs" }` when there are logs.
pub fn to_json<T: Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

/// [`to_json`] for a bare value, wrapped in a log-free [`Outcome`] first.
pub fn value_to_json<T: Serialize>(value: T) -> Result<Value, String> {
    Outcome::new(value, Vec::new()).into_cli_compatible_json()
}

/// Serialize only `outcome.value` and drop its logs (no log envelope). A
/// serde failure becomes `"{err_prefix}{serde error}"`, so each caller keeps
/// its own error copy; `""` passes the serde message through unchanged.
pub fn outcome_value_to_json<T: Serialize>(
    outcome: Outcome<T>,
    err_prefix: &str,
) -> Result<Value, String> {
    serde_json::to_value(outcome.value).map_err(|e| format!("{err_prefix}{e}"))
}

/// A short id (8 hex chars) that ties one controller call's log lines
/// together.
pub fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

// ── FieldSchema builders ─────────────────────────────────────────────────────

/// A required `TypeSchema::Json` field, most often a controller output.
pub fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

/// A required `TypeSchema::String` field.
pub fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

/// An optional `Option<String>` field.
pub fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

#[cfg(test)]
#[path = "params_tests.rs"]
mod tests;
