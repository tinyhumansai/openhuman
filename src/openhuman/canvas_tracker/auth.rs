use std::collections::HashMap;

use crate::openhuman::config::Config;
use crate::openhuman::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};
use crate::rpc::RpcOutcome;

use super::types::CANVAS_TRACKER_PROVIDER;

pub async fn store_canvas_token(
    config: &Config,
    token: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("canvas token must not be empty".to_string());
    }
    tracing::debug!(
        len = trimmed.len(),
        "[canvas_tracker] storing token (redacted)"
    );
    let auth = AuthService::from_config(config);
    auth.store_provider_token(
        CANVAS_TRACKER_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        trimmed,
        HashMap::new(),
        true,
    )
    .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "stored": true }),
        "canvas token stored",
    ))
}

pub fn get_canvas_token(config: &Config) -> Result<Option<String>, String> {
    let auth = AuthService::from_config(config);
    auth.get_provider_bearer_token(CANVAS_TRACKER_PROVIDER, None)
        .map(|value| {
            value
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty())
        })
        .map_err(|e| e.to_string())
}

pub async fn clear_canvas_token(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(CANVAS_TRACKER_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "removed": removed }),
        "canvas token cleared",
    ))
}
