//! Tauri command proxies for the standalone openhuman core process.

use openhuman_core::core_server::{
    BrowserSettingsUpdate, CommandResponse, ConfigSnapshot, GatewaySettingsUpdate,
    MemorySettingsUpdate, ModelSettingsUpdate, RuntimeFlags, RuntimeSettingsUpdate,
};
use openhuman_core::openhuman::{doctor, hardware, integrations, migration, onboard, service};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::Manager;

const DEFAULT_CORE_RPC_URL: &str = "http://127.0.0.1:7788/rpc";

fn params_none() -> serde_json::Value {
    serde_json::json!({})
}

async fn ensure_core(app: &tauri::AppHandle) -> Result<(), String> {
    let core = app
        .try_state::<crate::core_process::CoreProcessHandle>()
        .ok_or_else(|| "core process handle is not available".to_string())?;
    let handle: crate::core_process::CoreProcessHandle = (*core).clone();
    handle.ensure_running().await
}

async fn call_core<T: DeserializeOwned>(
    app: &tauri::AppHandle,
    method: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    ensure_core(app).await?;
    crate::core_rpc::call(method, params).await
}

async fn load_config_local() -> Result<openhuman_core::openhuman::config::Config, String> {
    let timeout_duration = std::time::Duration::from_secs(30);
    match tokio::time::timeout(
        timeout_duration,
        openhuman_core::openhuman::config::Config::load_or_init(),
    )
    .await
    {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentServerStatus {
    pub running: bool,
    pub url: String,
}

/// Return the current health snapshot as JSON.
#[tauri::command]
pub async fn openhuman_health_snapshot(
    app: tauri::AppHandle,
) -> Result<CommandResponse<serde_json::Value>, String> {
    call_core(&app, "openhuman.health_snapshot", params_none()).await
}

/// Return the default security policy info (autonomy config summary).
#[tauri::command]
pub async fn openhuman_security_policy_info(
    app: tauri::AppHandle,
) -> Result<CommandResponse<serde_json::Value>, String> {
    call_core(&app, "openhuman.security_policy_info", params_none()).await
}

/// Encrypt a secret using the openhuman SecretStore.
#[tauri::command]
pub async fn openhuman_encrypt_secret(
    app: tauri::AppHandle,
    plaintext: String,
) -> Result<CommandResponse<String>, String> {
    call_core(
        &app,
        "openhuman.encrypt_secret",
        serde_json::json!({ "plaintext": plaintext }),
    )
    .await
}

/// Decrypt a secret using the openhuman SecretStore.
#[tauri::command]
pub async fn openhuman_decrypt_secret(
    app: tauri::AppHandle,
    ciphertext: String,
) -> Result<CommandResponse<String>, String> {
    call_core(
        &app,
        "openhuman.decrypt_secret",
        serde_json::json!({ "ciphertext": ciphertext }),
    )
    .await
}

/// Return the full OpenHuman config snapshot for UI editing.
#[tauri::command]
pub async fn openhuman_get_config(
    app: tauri::AppHandle,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    call_core(&app, "openhuman.get_config", params_none()).await
}

/// Update model/provider settings.
#[tauri::command]
pub async fn openhuman_update_model_settings(
    app: tauri::AppHandle,
    update: ModelSettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    call_core(
        &app,
        "openhuman.update_model_settings",
        serde_json::json!(update),
    )
    .await
}

/// Update memory settings.
#[tauri::command]
pub async fn openhuman_update_memory_settings(
    app: tauri::AppHandle,
    update: MemorySettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    call_core(
        &app,
        "openhuman.update_memory_settings",
        serde_json::json!(update),
    )
    .await
}

/// Update gateway settings.
#[tauri::command]
pub async fn openhuman_update_gateway_settings(
    app: tauri::AppHandle,
    update: GatewaySettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    call_core(
        &app,
        "openhuman.update_gateway_settings",
        serde_json::json!(update),
    )
    .await
}

/// Update tunnel settings (full tunnel config).
#[tauri::command]
pub async fn openhuman_update_tunnel_settings(
    app: tauri::AppHandle,
    tunnel: openhuman_core::openhuman::config::TunnelConfig,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    call_core(
        &app,
        "openhuman.update_tunnel_settings",
        serde_json::json!(tunnel),
    )
    .await
}

/// Update runtime settings (skill execution backend).
#[tauri::command]
pub async fn openhuman_update_runtime_settings(
    app: tauri::AppHandle,
    update: RuntimeSettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    call_core(
        &app,
        "openhuman.update_runtime_settings",
        serde_json::json!(update),
    )
    .await
}

/// Update browser settings (Chrome/Chromium tool).
#[tauri::command]
pub async fn openhuman_update_browser_settings(
    app: tauri::AppHandle,
    update: BrowserSettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    call_core(
        &app,
        "openhuman.update_browser_settings",
        serde_json::json!(update),
    )
    .await
}

/// Read runtime flags that are controlled via environment variables.
#[tauri::command]
pub async fn openhuman_get_runtime_flags(
    app: tauri::AppHandle,
) -> Result<CommandResponse<RuntimeFlags>, String> {
    call_core(&app, "openhuman.get_runtime_flags", params_none()).await
}

/// Set browser allow-all flag for the current process.
#[tauri::command]
pub async fn openhuman_set_browser_allow_all(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<CommandResponse<RuntimeFlags>, String> {
    call_core(
        &app,
        "openhuman.set_browser_allow_all",
        serde_json::json!({ "enabled": enabled }),
    )
    .await
}

/// Send a single message to the OpenHuman agent and return the response text.
#[tauri::command]
pub async fn openhuman_agent_chat(
    app: tauri::AppHandle,
    message: String,
    provider_override: Option<String>,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<CommandResponse<String>, String> {
    call_core(
        &app,
        "openhuman.agent_chat",
        serde_json::json!({
            "message": message,
            "provider_override": provider_override,
            "model_override": model_override,
            "temperature": temperature,
        }),
    )
    .await
}

/// Run OpenHuman doctor checks and return a structured report.
#[tauri::command]
pub async fn openhuman_doctor_report(
    app: tauri::AppHandle,
) -> Result<CommandResponse<doctor::DoctorReport>, String> {
    call_core(&app, "openhuman.doctor_report", params_none()).await
}

/// Run model catalog probes for providers.
#[tauri::command]
pub async fn openhuman_doctor_models(
    app: tauri::AppHandle,
    provider_override: Option<String>,
    use_cache: Option<bool>,
) -> Result<CommandResponse<doctor::ModelProbeReport>, String> {
    call_core(
        &app,
        "openhuman.doctor_models",
        serde_json::json!({
            "provider_override": provider_override,
            "use_cache": use_cache,
        }),
    )
    .await
}

/// List integrations with status for the current config.
#[tauri::command]
pub async fn openhuman_list_integrations(
    app: tauri::AppHandle,
) -> Result<CommandResponse<Vec<integrations::IntegrationInfo>>, String> {
    call_core(&app, "openhuman.list_integrations", params_none()).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IntegrationInfoParams {
    name: String,
}

/// Get details for a single integration.
#[tauri::command]
pub async fn openhuman_get_integration_info(
    app: tauri::AppHandle,
    name: String,
) -> Result<CommandResponse<integrations::IntegrationInfo>, String> {
    let params = IntegrationInfoParams { name };
    call_core(
        &app,
        "openhuman.get_integration_info",
        serde_json::json!(params),
    )
    .await
}

/// Refresh the model catalog for a provider (or default provider).
#[tauri::command]
pub async fn openhuman_models_refresh(
    app: tauri::AppHandle,
    provider_override: Option<String>,
    force: Option<bool>,
) -> Result<CommandResponse<onboard::ModelRefreshResult>, String> {
    call_core(
        &app,
        "openhuman.models_refresh",
        serde_json::json!({
            "provider_override": provider_override,
            "force": force,
        }),
    )
    .await
}

/// Migrate OpenClaw memory into the current OpenHuman workspace.
#[tauri::command]
pub async fn openhuman_migrate_openclaw(
    app: tauri::AppHandle,
    source_workspace: Option<String>,
    dry_run: Option<bool>,
) -> Result<CommandResponse<migration::MigrationReport>, String> {
    call_core(
        &app,
        "openhuman.migrate_openclaw",
        serde_json::json!({
            "source_workspace": source_workspace,
            "dry_run": dry_run,
        }),
    )
    .await
}

/// Discover connected hardware devices (feature-gated).
#[tauri::command]
pub async fn openhuman_hardware_discover(
    app: tauri::AppHandle,
) -> Result<CommandResponse<Vec<hardware::DiscoveredDevice>>, String> {
    call_core(&app, "openhuman.hardware_discover", params_none()).await
}

/// Introspect a device path (feature-gated).
#[tauri::command]
pub async fn openhuman_hardware_introspect(
    app: tauri::AppHandle,
    path: String,
) -> Result<CommandResponse<hardware::HardwareIntrospect>, String> {
    call_core(
        &app,
        "openhuman.hardware_introspect",
        serde_json::json!({ "path": path }),
    )
    .await
}

/// Return whether the local core agent server is reachable.
#[tauri::command]
pub async fn openhuman_agent_server_status() -> Result<CommandResponse<AgentServerStatus>, String> {
    let url = std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| DEFAULT_CORE_RPC_URL.to_string());
    let running = crate::core_rpc::ping().await;
    Ok(CommandResponse {
        result: AgentServerStatus { running, url },
        logs: vec!["agent server status checked".to_string()],
    })
}

/// Install the OpenHuman daemon service.
#[tauri::command]
pub async fn openhuman_service_install(
    _app: tauri::AppHandle,
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    let config = load_config_local().await?;
    service::install(&config)
        .map(|status| CommandResponse {
            result: status,
            logs: vec!["service install completed".to_string()],
        })
        .map_err(|e| e.to_string())
}

/// Start the OpenHuman daemon service.
#[tauri::command]
pub async fn openhuman_service_start(
    _app: tauri::AppHandle,
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    let config = load_config_local().await?;
    service::start(&config)
        .map(|status| CommandResponse {
            result: status,
            logs: vec!["service start completed".to_string()],
        })
        .map_err(|e| e.to_string())
}

/// Stop the OpenHuman daemon service.
#[tauri::command]
pub async fn openhuman_service_stop(
    app: tauri::AppHandle,
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    let config = load_config_local().await?;
    let status = service::stop(&config).map_err(|e| e.to_string())?;

    // Also stop any locally managed core process in this Tauri app.
    if let Some(core) = app.try_state::<crate::core_process::CoreProcessHandle>() {
        let core_handle: crate::core_process::CoreProcessHandle = (*core).clone();
        core_handle.shutdown().await;
    }

    Ok(CommandResponse {
        result: status,
        logs: vec!["service stop completed".to_string()],
    })
}

/// Get the OpenHuman daemon service status.
#[tauri::command]
pub async fn openhuman_service_status(
    _app: tauri::AppHandle,
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    let config = load_config_local().await?;
    service::status(&config)
        .map(|status| CommandResponse {
            result: status,
            logs: vec!["service status fetched".to_string()],
        })
        .map_err(|e| e.to_string())
}

/// Uninstall the OpenHuman daemon service.
#[tauri::command]
pub async fn openhuman_service_uninstall(
    app: tauri::AppHandle,
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    let config = load_config_local().await?;
    let status = service::uninstall(&config).map_err(|e| e.to_string())?;

    if let Some(core) = app.try_state::<crate::core_process::CoreProcessHandle>() {
        let core_handle: crate::core_process::CoreProcessHandle = (*core).clone();
        core_handle.shutdown().await;
    }

    Ok(CommandResponse {
        result: status,
        logs: vec!["service uninstall completed".to_string()],
    })
}
