use crate::core_process::CoreProcessHandle;
use crate::daemon_host_config::{self, DaemonHostConfig};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceState {
    Running,
    Stopped,
    NotInstalled,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub unit_path: Option<std::path::PathBuf>,
    pub label: String,
    pub details: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcCommandResponse<T> {
    result: T,
}

async fn ensure_core_running(app: &AppHandle) -> Result<(), String> {
    let core = app
        .try_state::<CoreProcessHandle>()
        .ok_or_else(|| "core process handle is not available".to_string())?;
    let handle: CoreProcessHandle = (*core).clone();
    handle.ensure_running().await
}

async fn call_service_method(app: &AppHandle, method: &str) -> Result<ServiceStatus, String> {
    ensure_core_running(app).await?;
    let response =
        crate::core_rpc::call::<RpcCommandResponse<ServiceStatus>>(method, serde_json::json!({}))
            .await?;
    Ok(response.result)
}

#[tauri::command]
pub async fn openhuman_get_daemon_host_config(app: AppHandle) -> Result<DaemonHostConfig, String> {
    Ok(daemon_host_config::load(&app).await)
}

#[tauri::command]
pub async fn openhuman_set_daemon_host_config(
    app: AppHandle,
    show_tray: bool,
) -> Result<DaemonHostConfig, String> {
    let mut cfg = daemon_host_config::load(&app).await;
    cfg.show_tray = show_tray;
    daemon_host_config::save(&app, &cfg).await?;
    Ok(cfg)
}

#[tauri::command]
pub async fn openhuman_service_install(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_install").await
}

#[tauri::command]
pub async fn openhuman_service_start(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_start").await
}

#[tauri::command]
pub async fn openhuman_service_stop(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_stop").await
}

#[tauri::command]
pub async fn openhuman_service_status(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_status").await
}

#[tauri::command]
pub async fn openhuman_service_uninstall(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_uninstall").await
}
