use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{Map, Value};

use crate::core::all;
use crate::core_server::types::{AppState, RpcError, RpcFailure, RpcRequest, RpcSuccess};

pub async fn rpc_handler(State(state): State<AppState>, Json(req): Json<RpcRequest>) -> Response {
    let id = req.id.clone();
    match invoke_method(state, req.method.as_str(), req.params).await {
        Ok(value) => (
            StatusCode::OK,
            Json(RpcSuccess {
                jsonrpc: "2.0",
                id,
                result: value,
            }),
        )
            .into_response(),
        Err(message) => (
            StatusCode::OK,
            Json(RpcFailure {
                jsonrpc: "2.0",
                id,
                error: RpcError {
                    code: -32000,
                    message,
                    data: None,
                },
            }),
        )
            .into_response(),
    }
}

pub async fn invoke_method(
    state: AppState,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    if let Some(schema) = all::schema_for_rpc_method(method) {
        let params_obj = params_to_object(params)?;
        all::validate_params(&schema, &params_obj)?;
        if let Some(result) = all::try_invoke_registered_rpc(method, params_obj).await {
            return result;
        }
        return Err(format!("registered schema has no handler: {method}"));
    }

    crate::core_server::dispatch::dispatch(state, method, params).await
}

fn params_to_object(params: Value) -> Result<Map<String, Value>, String> {
    match params {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        other => Err(format!(
            "invalid params: expected object or null, got {}",
            type_name(&other)
        )),
    }
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

pub fn parse_json_params(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid JSON params: {e}"))
}

pub fn default_state() -> AppState {
    AppState {
        core_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}
