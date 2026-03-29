use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

#[allow(dead_code)]
static DESKTOP_APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

#[allow(dead_code)]
static DESKTOP_RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

#[allow(dead_code)]
pub fn init_desktop_app_handle(handle: tauri::AppHandle) {
    let _ = DESKTOP_APP_HANDLE.set(handle);
}

#[allow(dead_code)]
pub fn desktop_app_handle() -> Result<tauri::AppHandle, String> {
    DESKTOP_APP_HANDLE
        .get()
        .cloned()
        .ok_or_else(|| "desktop app handle not set".to_string())
}

#[allow(dead_code)]
pub fn init_desktop_resource_dir(dir: PathBuf) {
    let _ = DESKTOP_RESOURCE_DIR.set(dir);
}

#[allow(dead_code)]
pub fn desktop_resource_dir() -> Option<PathBuf> {
    DESKTOP_RESOURCE_DIR.get().cloned()
}

pub async fn load_openhuman_config() -> Result<Config, String> {
    crate::openhuman::config::rpc::load_config_with_timeout().await
}

pub fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

/// Maps a domain [`RpcOutcome`](crate::rpc::RpcOutcome) into a JSON-RPC [`InvocationResult`].
pub fn rpc_invocation_from_outcome<T: Serialize>(
    o: RpcOutcome<T>,
) -> Result<super::types::InvocationResult, String> {
    super::types::InvocationResult::with_logs(o.value, o.logs)
}

/// Wraps a domain [`RpcOutcome`] the same way as JSON-RPC / [`super::types::invocation_to_rpc_json`] for CLI output.
pub fn rpc_outcome_to_cli_json<T: Serialize>(
    outcome: RpcOutcome<T>,
) -> Result<serde_json::Value, String> {
    Ok(super::types::invocation_to_rpc_json(
        rpc_invocation_from_outcome(outcome)?,
    ))
}

pub async fn rpc_outcome_fut_to_cli_json<T: Serialize>(
    fut: impl std::future::Future<Output = Result<RpcOutcome<T>, String>>,
) -> Result<serde_json::Value, String> {
    rpc_outcome_to_cli_json(fut.await?)
}
