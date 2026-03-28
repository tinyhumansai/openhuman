use serde_json::json;

use crate::core_server::types::{AppState, InvocationResult};

pub fn try_dispatch(
    state: &AppState,
    method: &str,
    _params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "core.ping" => Some(InvocationResult::ok(json!({ "ok": true }))),
        "core.version" => Some(InvocationResult::ok(json!({ "version": state.core_version }))),
        _ => None,
    }
}
