//! `GET /health` and `GET /schema`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::core_host::core::types::AppState;

/// Handler for the health check endpoint.
///
/// Liveness is granular (#3312): a single degraded *background* component
/// (scheduler, channels, update_checker, …) no longer 503s the whole container.
/// `/health` returns 503 only when a *critical* component is unhealthy (see
/// `health::CRITICAL_COMPONENTS`); otherwise it returns 200 — with a `degraded`
/// flag and per-component buckets in the body so readiness probes and operators
/// can still see partial failures.
pub(super) async fn health_handler() -> impl IntoResponse {
    let snapshot = crate::core_host::platform::health::snapshot();
    let verdict = crate::core_host::platform::health::verdict(&snapshot);

    let status = if verdict.healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    // Augment the snapshot body with the verdict so the components map stays
    // backward-compatible while exposing overall liveness/readiness.
    let mut body = serde_json::to_value(&snapshot).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = body.as_object_mut() {
        obj.insert("healthy".to_string(), serde_json::json!(verdict.healthy));
        obj.insert("degraded".to_string(), serde_json::json!(verdict.degraded));
        obj.insert(
            "critical_unhealthy".to_string(),
            serde_json::json!(verdict.critical_unhealthy),
        );
        obj.insert(
            "degraded_components".to_string(),
            serde_json::json!(verdict.degraded_components),
        );
    }

    tracing::debug!(
        "[health] status={} components={} healthy={} degraded={} critical_unhealthy={:?} degraded_components={:?}",
        status.as_u16(),
        snapshot.components.len(),
        verdict.healthy,
        verdict.degraded,
        verdict.critical_unhealthy,
        verdict.degraded_components,
    );

    (status, Json(body))
}

/// Handler for the schema discovery endpoint.
pub(super) async fn schema_handler(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, Json(build_http_schema_dump())).into_response()
}

/// JSON-serializable wrapper for the entire RPC schema dump.
#[derive(Serialize)]
struct HttpSchemaDump {
    /// List of all available RPC methods and their schemas.
    methods: Vec<HttpMethodSchema>,
}

/// JSON-serializable schema for a single RPC method.
#[derive(Serialize)]
struct HttpMethodSchema {
    /// Fully qualified JSON-RPC method name.
    method: String,
    /// Namespace of the function.
    namespace: String,
    /// Function name within the namespace.
    function: String,
    /// Human-readable description of what the method does.
    description: String,
    /// List of input parameters.
    inputs: Vec<crate::core_host::core::FieldSchema>,
    /// List of output fields.
    outputs: Vec<crate::core_host::core::FieldSchema>,
}

/// Aggregates schemas from all registered controllers into a single dump.
///
/// Also includes built-in core methods like `core.ping` and `core.version`.
fn build_http_schema_dump() -> HttpSchemaDump {
    let mut methods: Vec<HttpMethodSchema> = crate::core_host::core::all::all_http_method_schemas()
        .into_iter()
        .map(|method| HttpMethodSchema {
            method: method.method,
            namespace: method.namespace.to_string(),
            function: method.function.to_string(),
            description: method.description.to_string(),
            inputs: method.inputs,
            outputs: method.outputs,
        })
        .collect();

    // Sort methods alphabetically for consistent output.
    methods.sort_by(|a, b| a.method.cmp(&b.method));

    HttpSchemaDump { methods }
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;
