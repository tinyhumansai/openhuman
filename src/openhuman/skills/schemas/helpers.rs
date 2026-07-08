//! Internal utility helpers shared across all workflow schema handlers.
//!
//! Provides config / workspace resolution with graceful fallbacks, and the
//! thin `deserialize_params` / `to_json` adapters used by every handler.

use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

// ── Config / workspace resolution ────────────────────────────────────────────

/// Resolve the active [`Config`]. Falls back to `Config::default()` with a
/// best-effort workspace directory if the persisted load times out or errors,
/// so headless diagnostics still work in partially-initialized environments.
pub(super) async fn resolve_config() -> Config {
    match tokio::time::timeout(std::time::Duration::from_secs(30), Config::load_or_init()).await {
        Ok(Ok(cfg)) => cfg,
        Ok(Err(err)) => {
            tracing::debug!(
                error = %err,
                "[skills][rpc] config load failed; falling back to default config"
            );
            fallback_config()
        }
        Err(_) => {
            tracing::debug!("[skills][rpc] config load timed out; falling back to default config");
            fallback_config()
        }
    }
}

fn fallback_config() -> Config {
    Config {
        workspace_dir: fallback_workspace_dir(),
        ..Default::default()
    }
}

/// Resolve the active workspace directory. Falls back to the runtime default
/// if the persisted config fails to load so the CLI and headless diagnostics
/// still work in partially-initialized environments.
pub(crate) async fn resolve_workspace_dir() -> PathBuf {
    match tokio::time::timeout(std::time::Duration::from_secs(30), Config::load_or_init()).await {
        Ok(Ok(cfg)) => cfg.workspace_dir,
        Ok(Err(err)) => {
            tracing::debug!(
                error = %err,
                "[skills][rpc] config load failed; falling back to default workspace"
            );
            fallback_workspace_dir()
        }
        Err(_) => {
            tracing::debug!(
                "[skills][rpc] config load timed out; falling back to default workspace"
            );
            fallback_workspace_dir()
        }
    }
}

fn fallback_workspace_dir() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| PathBuf::from(".openhuman"))
        .join("workspace")
}

// ── Serde adapters ────────────────────────────────────────────────────────────

pub(super) fn deserialize_params<T: DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

pub(super) fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
