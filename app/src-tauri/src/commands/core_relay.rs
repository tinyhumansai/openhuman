use crate::core_process::CoreProcessHandle;
use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreRpcRelayRequest {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub service_managed: bool,
}

#[tauri::command]
pub async fn core_rpc_relay(
    app: tauri::AppHandle,
    request: CoreRpcRelayRequest,
) -> Result<Value, String> {
    let _ = request.service_managed;

    let core = app
        .try_state::<CoreProcessHandle>()
        .ok_or_else(|| "core process handle is not available".to_string())?;
    let handle: CoreProcessHandle = (*core).clone();
    handle.ensure_running().await?;

    crate::core_rpc::call::<Value>(&request.method, request.params).await
}

#[tauri::command]
pub fn core_rpc_url() -> String {
    crate::core_rpc::resolved_rpc_url()
}
