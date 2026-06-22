//! AgentBox HTTP sub-router.
//!
//! Exposes `POST /run` and `GET /jobs/{job_id}` per the AgentBox marketplace
//! contract. Mounted onto the core HTTP router by
//! `core::jsonrpc::build_core_http_router` when `OPENHUMAN_AGENTBOX_MODE=1`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use std::time::Duration;

use super::invoker::SharedInvoker;
use super::ops::submit_run;
use super::store::JobStore;
use super::types::{RunRequest, RunResponse};

#[derive(Clone)]
struct HttpState {
    store: JobStore,
    invoker: SharedInvoker,
    job_timeout: Duration,
}

/// Build the AgentBox sub-router.
///
/// `job_timeout` caps how long any single agent invocation may run before
/// the worker forces it to `failed`.
pub fn router(store: JobStore, invoker: SharedInvoker, job_timeout: Duration) -> Router {
    Router::new()
        .route("/run", post(post_run))
        .route("/jobs/{job_id}", get(get_job))
        .with_state(HttpState {
            store,
            invoker,
            job_timeout,
        })
}

async fn post_run(
    State(state): State<HttpState>,
    body: Result<Json<RunRequest>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(j) => j,
        Err(rej) => {
            log::debug!("[agentbox] /run rejected: JSON parse/validation failed: {rej}");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": rej.to_string() })),
            )
                .into_response();
        }
    };

    if req.payload.message.trim().is_empty() {
        log::debug!("[agentbox] /run rejected: empty payload.message");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "payload.message must be a non-empty string" })),
        )
            .into_response();
    }

    let id = submit_run(
        state.store.clone(),
        state.invoker.clone(),
        req.payload,
        state.job_timeout,
    );
    log::info!("[agentbox] /run accepted job_id={}", id);
    (StatusCode::ACCEPTED, Json(RunResponse { job_id: id })).into_response()
}

async fn get_job(
    State(state): State<HttpState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.store.get(&job_id) {
        Some(view) => (StatusCode::OK, Json(view)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "job not found" })),
        )
            .into_response(),
    }
}
