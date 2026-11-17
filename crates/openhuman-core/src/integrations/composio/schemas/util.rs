//! Small param-decoding and response-shaping helpers shared by every
//! handler in `schemas/handlers_*.rs`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

pub(super) fn read_required<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

/// Read a required `String` parameter and reject blank / whitespace-only
/// input at the RPC boundary instead of letting it reach the backend.
/// Returns the trimmed value.
pub(super) fn read_required_non_empty(
    params: &Map<String, Value>,
    key: &str,
) -> Result<String, String> {
    let raw = read_required::<String>(params, key)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("'{key}' must not be empty"));
    }
    Ok(trimmed.to_string())
}

pub(super) fn read_optional<T: DeserializeOwned>(
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
