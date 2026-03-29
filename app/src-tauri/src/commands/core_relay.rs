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

async fn invoke_core_cli_call(method: &str, params: Value) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to resolve current exe: {e}"))?;
    let params_json =
        serde_json::to_string(&params).map_err(|e| format!("failed to serialize params: {e}"))?;

    let output = tokio::process::Command::new(exe)
        .arg("core")
        .arg("call")
        .arg("--method")
        .arg(method)
        .arg("--params")
        .arg(params_json)
        .output()
        .await
        .map_err(|e| format!("failed to run core cli call {method}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(format!(
            "core cli call {method} failed with status {}. stdout: {} stderr: {}",
            output.status, stdout, stderr
        ));
    }

    Ok(())
}

pub(crate) async fn ensure_service_managed_core_running() -> Result<(), String> {
    if crate::core_rpc::ping().await {
        return Ok(());
    }

    let _ = invoke_core_cli_call("openhuman.service_install", serde_json::json!({})).await;
    let _ = invoke_core_cli_call("openhuman.service_start", serde_json::json!({})).await;

    for _ in 0..40 {
        if crate::core_rpc::ping().await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    Err(
        "OpenHuman Core daemon did not become ready. Confirm the background service is running."
            .to_string(),
    )
}

#[tauri::command]
pub async fn core_rpc_relay(
    app: tauri::AppHandle,
    request: CoreRpcRelayRequest,
) -> Result<Value, String> {
    if request.service_managed {
        ensure_service_managed_core_running().await?;
    } else {
        let core = app
            .try_state::<CoreProcessHandle>()
            .ok_or_else(|| "core process handle is not available".to_string())?;
        let handle: CoreProcessHandle = (*core).clone();
        handle.ensure_running().await?;
    }

    crate::core_rpc::call::<Value>(&request.method, request.params).await
}
