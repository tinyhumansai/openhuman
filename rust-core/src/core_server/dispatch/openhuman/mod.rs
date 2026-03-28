mod auth_socket;
mod config;
mod cron;
mod local_ai;
mod ops;
mod platform;

use crate::core_server::types::{AppState, InvocationResult};

pub async fn try_dispatch(
    _state: &AppState,
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    if let Some(r) = config::try_dispatch(method, params.clone()).await {
        return Some(r);
    }
    if let Some(r) = cron::try_dispatch(method, params.clone()).await {
        return Some(r);
    }
    if let Some(r) = local_ai::try_dispatch(method, params.clone()).await {
        return Some(r);
    }
    if let Some(r) = platform::try_dispatch(method, params.clone()).await {
        return Some(r);
    }
    if let Some(r) = ops::try_dispatch(method, params.clone()).await {
        return Some(r);
    }
    if let Some(r) = auth_socket::try_dispatch(method, params).await {
        return Some(r);
    }
    None
}
