//! Tauri command proxies for the standalone openhuman core process.

use openhuman_core::core_server::{
    AccessibilityStatus, AutocompleteCommitParams, AutocompleteCommitResult,
    AutocompleteSuggestParams, AutocompleteSuggestResult, BrowserSettingsUpdate, CaptureNowResult,
    CommandResponse, ConfigSnapshot, GatewaySettingsUpdate, InputActionParams, InputActionResult,
    MemorySettingsUpdate, ModelSettingsUpdate, PermissionStatus, RuntimeFlags,
    RuntimeSettingsUpdate, SessionStatus, StartSessionParams, StopSessionParams, VisionFlushResult,
    VisionRecentResult,
};
use openhuman_core::openhuman::{doctor, hardware, integrations, migration, onboard, service};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

#[cfg(desktop)]
use crate::services::notification_service::NotificationService;

const DEFAULT_CORE_RPC_URL: &str = "http://127.0.0.1:7788/rpc";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessibilityBridgeEvent {
    event: String,
    timestamp_ms: i64,
    details: serde_json::Value,
}

fn emit_accessibility_event(app: &tauri::AppHandle, event: &str, details: serde_json::Value) {
    let payload = AccessibilityBridgeEvent {
        event: event.to_string(),
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        details,
    };
    let _ = app.emit("openhuman:accessibility", &payload);

    #[cfg(desktop)]
    {
        let body = match event {
            "session_started" => "Accessibility session started",
            "session_stopped" => "Accessibility session stopped",
            "permissions_requested" => "Accessibility permission request opened",
            _ => "Accessibility automation event",
        };
        let _ = NotificationService::show(app, "Accessibility Automation", body);
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiStatus {
    pub state: String,
    pub model_id: String,
    pub chat_model_id: String,
    pub vision_model_id: String,
    pub embedding_model_id: String,
    pub stt_model_id: String,
    pub tts_voice_id: String,
    pub quantization: String,
    pub vision_state: String,
    pub embedding_state: String,
    pub stt_state: String,
    pub tts_state: String,
    pub provider: String,
    pub download_progress: Option<f32>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub download_speed_bps: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub warning: Option<String>,
    pub model_path: Option<String>,
    pub active_backend: String,
    pub backend_reason: Option<String>,
    pub last_latency_ms: Option<u64>,
    pub prompt_toks_per_sec: Option<f32>,
    pub gen_toks_per_sec: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiSuggestion {
    pub text: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiAssetStatus {
    pub state: String,
    pub id: String,
    pub provider: String,
    pub path: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiAssetsStatus {
    pub chat: LocalAiAssetStatus,
    pub vision: LocalAiAssetStatus,
    pub embedding: LocalAiAssetStatus,
    pub stt: LocalAiAssetStatus,
    pub tts: LocalAiAssetStatus,
    pub quantization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiEmbeddingResult {
    pub model_id: String,
    pub dimensions: usize,
    pub vectors: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiSpeechResult {
    pub text: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiTtsResult {
    pub output_path: String,
    pub voice_id: String,
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

/// Fetch accessibility automation status.
#[tauri::command]
pub async fn openhuman_accessibility_status(
    app: tauri::AppHandle,
) -> Result<CommandResponse<AccessibilityStatus>, String> {
    call_core(&app, "openhuman.accessibility_status", params_none()).await
}

/// Request accessibility-related permissions on macOS.
#[tauri::command]
pub async fn openhuman_accessibility_request_permissions(
    app: tauri::AppHandle,
) -> Result<CommandResponse<PermissionStatus>, String> {
    let response: CommandResponse<PermissionStatus> = call_core(
        &app,
        "openhuman.accessibility_request_permissions",
        params_none(),
    )
    .await?;
    emit_accessibility_event(
        &app,
        "permissions_requested",
        serde_json::json!(response.result),
    );
    Ok(response)
}

/// Start a bounded accessibility session with explicit consent.
#[tauri::command]
pub async fn openhuman_accessibility_start_session(
    app: tauri::AppHandle,
    params: StartSessionParams,
) -> Result<CommandResponse<SessionStatus>, String> {
    let response: CommandResponse<SessionStatus> = call_core(
        &app,
        "openhuman.accessibility_start_session",
        serde_json::json!(params),
    )
    .await?;
    emit_accessibility_event(&app, "session_started", serde_json::json!(response.result));
    Ok(response)
}

/// Stop the active accessibility session.
#[tauri::command]
pub async fn openhuman_accessibility_stop_session(
    app: tauri::AppHandle,
    params: Option<StopSessionParams>,
) -> Result<CommandResponse<SessionStatus>, String> {
    let response: CommandResponse<SessionStatus> = call_core(
        &app,
        "openhuman.accessibility_stop_session",
        serde_json::json!(params.unwrap_or(StopSessionParams { reason: None })),
    )
    .await?;
    emit_accessibility_event(&app, "session_stopped", serde_json::json!(response.result));
    Ok(response)
}

/// Force an immediate capture sample from the accessibility runtime.
#[tauri::command]
pub async fn openhuman_accessibility_capture_now(
    app: tauri::AppHandle,
) -> Result<CommandResponse<CaptureNowResult>, String> {
    call_core(&app, "openhuman.accessibility_capture_now", params_none()).await
}

/// Execute a validated input action in an active accessibility session.
#[tauri::command]
pub async fn openhuman_accessibility_input_action(
    app: tauri::AppHandle,
    params: InputActionParams,
) -> Result<CommandResponse<InputActionResult>, String> {
    let response: CommandResponse<InputActionResult> = call_core(
        &app,
        "openhuman.accessibility_input_action",
        serde_json::json!(params),
    )
    .await?;
    if response.result.accepted {
        emit_accessibility_event(&app, "input_action", serde_json::json!(response.result));
    }
    Ok(response)
}

/// Generate autocomplete suggestions from captured typing context.
#[tauri::command]
pub async fn openhuman_accessibility_autocomplete_suggest(
    app: tauri::AppHandle,
    params: Option<AutocompleteSuggestParams>,
) -> Result<CommandResponse<AutocompleteSuggestResult>, String> {
    call_core(
        &app,
        "openhuman.accessibility_autocomplete_suggest",
        serde_json::json!(params.unwrap_or(AutocompleteSuggestParams {
            context: None,
            max_results: None,
        })),
    )
    .await
}

/// Commit an autocomplete suggestion into the runtime context.
#[tauri::command]
pub async fn openhuman_accessibility_autocomplete_commit(
    app: tauri::AppHandle,
    params: AutocompleteCommitParams,
) -> Result<CommandResponse<AutocompleteCommitResult>, String> {
    call_core(
        &app,
        "openhuman.accessibility_autocomplete_commit",
        serde_json::json!(params),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_accessibility_vision_recent(
    app: tauri::AppHandle,
    limit: Option<usize>,
) -> Result<CommandResponse<VisionRecentResult>, String> {
    call_core(
        &app,
        "openhuman.accessibility_vision_recent",
        serde_json::json!({ "limit": limit }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_accessibility_vision_flush(
    app: tauri::AppHandle,
) -> Result<CommandResponse<VisionFlushResult>, String> {
    call_core(&app, "openhuman.accessibility_vision_flush", params_none()).await
}

#[tauri::command]
pub async fn openhuman_local_ai_status(
    app: tauri::AppHandle,
) -> Result<CommandResponse<LocalAiStatus>, String> {
    call_core(&app, "openhuman.local_ai_status", params_none()).await
}

#[tauri::command]
pub async fn openhuman_local_ai_download(
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<CommandResponse<LocalAiStatus>, String> {
    call_core(
        &app,
        "openhuman.local_ai_download",
        serde_json::json!({ "force": force }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_summarize(
    app: tauri::AppHandle,
    text: String,
    max_tokens: Option<u32>,
) -> Result<CommandResponse<String>, String> {
    call_core(
        &app,
        "openhuman.local_ai_summarize",
        serde_json::json!({
            "text": text,
            "max_tokens": max_tokens
        }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_suggest_questions(
    app: tauri::AppHandle,
    context: Option<String>,
    lines: Option<Vec<String>>,
) -> Result<CommandResponse<Vec<LocalAiSuggestion>>, String> {
    call_core(
        &app,
        "openhuman.local_ai_suggest_questions",
        serde_json::json!({
            "context": context,
            "lines": lines
        }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_prompt(
    app: tauri::AppHandle,
    prompt: String,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<CommandResponse<String>, String> {
    call_core(
        &app,
        "openhuman.local_ai_prompt",
        serde_json::json!({
            "prompt": prompt,
            "max_tokens": max_tokens,
            "no_think": no_think
        }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_vision_prompt(
    app: tauri::AppHandle,
    prompt: String,
    image_refs: Vec<String>,
    max_tokens: Option<u32>,
) -> Result<CommandResponse<String>, String> {
    call_core(
        &app,
        "openhuman.local_ai_vision_prompt",
        serde_json::json!({
            "prompt": prompt,
            "image_refs": image_refs,
            "max_tokens": max_tokens
        }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_embed(
    app: tauri::AppHandle,
    inputs: Vec<String>,
) -> Result<CommandResponse<LocalAiEmbeddingResult>, String> {
    call_core(
        &app,
        "openhuman.local_ai_embed",
        serde_json::json!({
            "inputs": inputs
        }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_transcribe(
    app: tauri::AppHandle,
    audio_path: String,
) -> Result<CommandResponse<LocalAiSpeechResult>, String> {
    call_core(
        &app,
        "openhuman.local_ai_transcribe",
        serde_json::json!({
            "audio_path": audio_path
        }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_tts(
    app: tauri::AppHandle,
    text: String,
    output_path: Option<String>,
) -> Result<CommandResponse<LocalAiTtsResult>, String> {
    call_core(
        &app,
        "openhuman.local_ai_tts",
        serde_json::json!({
            "text": text,
            "output_path": output_path
        }),
    )
    .await
}

#[tauri::command]
pub async fn openhuman_local_ai_assets_status(
    app: tauri::AppHandle,
) -> Result<CommandResponse<LocalAiAssetsStatus>, String> {
    call_core(&app, "openhuman.local_ai_assets_status", params_none()).await
}

#[tauri::command]
pub async fn openhuman_local_ai_download_asset(
    app: tauri::AppHandle,
    capability: String,
) -> Result<CommandResponse<LocalAiAssetsStatus>, String> {
    call_core(
        &app,
        "openhuman.local_ai_download_asset",
        serde_json::json!({ "capability": capability }),
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
