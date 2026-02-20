//! Health and metrics endpoints.

use crate::alphahuman::gateway::state::AppState;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Json},
};

/// Prometheus content type for text exposition format.
pub const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// GET /health — always public (no secrets leaked).
pub async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    let body = serde_json::json!({
        "status": "ok",
        "paired": state.pairing.is_paired(),
        "runtime": crate::alphahuman::health::snapshot_json(),
    });
    Json(body)
}

/// GET /metrics — Prometheus text exposition format.
pub async fn handle_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let body = if let Some(prom) = state
        .observer
        .as_ref()
        .as_any()
        .downcast_ref::<crate::alphahuman::observability::PrometheusObserver>()
    {
        prom.encode()
    } else {
        String::from(
            "# Prometheus backend not enabled. Set [observability] backend = \"prometheus\" in config.\n",
        )
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
}
