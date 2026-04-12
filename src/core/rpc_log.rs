use serde_json::Value;

/// Formats a JSON-RPC request ID into a human-readable string.
///
/// Handles different JSON types (String, Number, Null) to ensure consistent
/// output in log messages.
pub fn format_request_id(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// Redacts sensitive keys from a JSON parameters object before logging.
///
/// This is used to prevent accidental leakage of API keys, tokens, and passwords
/// in debug logs.
pub fn redact_params_for_log(params: &Value) -> Value {
    redact_value(params)
}

/// Produces a short summary of a JSON value, useful for high-level logging.
///
/// Instead of printing a potentially massive object/array, it returns a
/// string like `object(keys=foo,bar)` or `array(len=10)`.
pub fn summarize_rpc_result(result: &Value) -> String {
    match result {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            format!("object(keys={})", keys.join(","))
        }
        Value::Array(items) => format!("array(len={})", items.len()),
        Value::String(s) => format!("string(len={})", s.len()),
        Value::Bool(b) => format!("bool({b})"),
        Value::Number(n) => format!("number({n})"),
        Value::Null => "null".to_string(),
    }
}

/// Redacts sensitive keys from a JSON result object before trace logging.
pub fn redact_result_for_trace(result: &Value) -> Value {
    redact_value(result)
}

/// Recursively redacts sensitive information from a JSON value.
///
/// It traverses objects and arrays, replacing values of keys that match
/// [`is_sensitive_key`] with `[REDACTED]`.
fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if is_sensitive_key(k) {
                    out.insert(k.clone(), Value::String("[REDACTED]".to_string()));
                } else {
                    out.insert(k.clone(), redact_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

/// Returns true if a key name is considered sensitive (e.g., "api_key", "password").
fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key,
        "api_key"
            | "apikey"
            | "token"
            | "access_token"
            | "refresh_token"
            | "authorization"
            | "password"
            | "secret"
            | "client_secret"
    )
}
