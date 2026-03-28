use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::health;
use crate::openhuman::local_ai::{self, Suggestion};
use crate::openhuman::security::{SecretStore, SecurityPolicy};
use crate::openhuman::{
    accessibility, doctor, hardware, integrations, migration, onboard, service,
};

pub use crate::openhuman::accessibility::{
    AccessibilityStatus, AutocompleteCommitParams, AutocompleteCommitResult,
    AutocompleteSuggestParams, AutocompleteSuggestResult, CaptureNowResult, InputActionParams,
    InputActionResult, PermissionStatus, SessionStatus, StartSessionParams, StopSessionParams,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse<T> {
    pub result: T,
    pub logs: Vec<String>,
}

fn command_response<T>(result: T, logs: Vec<String>) -> CommandResponse<T> {
    CommandResponse { result, logs }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RpcSuccess {
    jsonrpc: &'static str,
    id: serde_json::Value,
    result: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RpcFailure {
    jsonrpc: &'static str,
    id: serde_json::Value,
    error: RpcError,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<serde_json::Value>,
}

#[derive(Clone)]
struct AppState {
    core_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub config: serde_json::Value,
    pub workspace_dir: String,
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettingsUpdate {
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySettingsUpdate {
    pub backend: Option<String>,
    pub auto_save: Option<bool>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySettingsUpdate {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub require_pairing: Option<bool>,
    pub allow_public_bind: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSettingsUpdate {
    pub kind: Option<String>,
    pub reasoning_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSettingsUpdate {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeFlags {
    pub browser_allow_all: bool,
    pub log_prompts: bool,
}

#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    provider_override: Option<String>,
    model_override: Option<String>,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DoctorModelsParams {
    provider_override: Option<String>,
    use_cache: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct IntegrationInfoParams {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ModelsRefreshParams {
    provider_override: Option<String>,
    force: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MigrateOpenClawParams {
    source_workspace: Option<String>,
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct HardwareIntrospectParams {
    path: String,
}

#[derive(Debug, Deserialize)]
struct EncryptSecretParams {
    plaintext: String,
}

#[derive(Debug, Deserialize)]
struct DecryptSecretParams {
    ciphertext: String,
}

#[derive(Debug, Deserialize)]
struct SetBrowserAllowAllParams {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct LocalAiDownloadParams {
    force: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LocalAiSummarizeParams {
    text: String,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LocalAiSuggestParams {
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    lines: Option<Vec<String>>,
}

async fn load_openhuman_config() -> Result<Config, String> {
    let timeout_duration = std::time::Duration::from_secs(30);
    match tokio::time::timeout(timeout_duration, Config::load_or_init()).await {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

fn snapshot_config(config: &Config) -> Result<ConfigSnapshot, String> {
    let value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    Ok(ConfigSnapshot {
        config: value,
        workspace_dir: config.workspace_dir.display().to_string(),
        config_path: config.config_path.display().to_string(),
    })
}

fn env_flag_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn secret_store_for_config(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new(&data_dir, true)
}

fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

fn rpc_error_response(id: serde_json::Value, code: i64, message: String) -> Response {
    (
        StatusCode::OK,
        Json(RpcFailure {
            jsonrpc: "2.0",
            id,
            error: RpcError {
                code,
                message,
                data: None,
            },
        }),
    )
        .into_response()
}

fn to_rpc_success(id: serde_json::Value, value: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        Json(RpcSuccess {
            jsonrpc: "2.0",
            id,
            result: value,
        }),
    )
        .into_response()
}

fn to_json_value<T: Serialize>(value: T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

async fn rpc_handler(State(state): State<AppState>, Json(req): Json<RpcRequest>) -> Response {
    let id = req.id.clone();

    let result = dispatch(state, req.method.as_str(), req.params).await;

    match result {
        Ok(value) => to_rpc_success(id, value),
        Err(message) => rpc_error_response(id, -32000, message),
    }
}

async fn dispatch(
    state: AppState,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match method {
        "core.ping" => to_json_value(json!({ "ok": true })),
        "core.version" => to_json_value(json!({ "version": state.core_version })),

        "openhuman.health_snapshot" => to_json_value(command_response(
            health::snapshot_json(),
            vec!["health_snapshot requested".to_string()],
        )),

        "openhuman.security_policy_info" => {
            let policy = SecurityPolicy::default();
            let payload = json!({
                "autonomy": policy.autonomy,
                "workspace_only": policy.workspace_only,
                "allowed_commands": policy.allowed_commands,
                "max_actions_per_hour": policy.max_actions_per_hour,
                "require_approval_for_medium_risk": policy.require_approval_for_medium_risk,
                "block_high_risk_commands": policy.block_high_risk_commands,
            });
            to_json_value(command_response(
                payload,
                vec!["security_policy_info computed".to_string()],
            ))
        }

        "openhuman.get_config" => {
            let config = load_openhuman_config().await?;
            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "config loaded from {}",
                    config.config_path.display()
                )],
            ))
        }

        "openhuman.update_model_settings" => {
            let update: ModelSettingsUpdate = parse_params(params)?;
            let mut config = load_openhuman_config().await?;
            if let Some(api_key) = update.api_key {
                config.api_key = if api_key.trim().is_empty() {
                    None
                } else {
                    Some(api_key)
                };
            }
            if let Some(api_url) = update.api_url {
                config.api_url = if api_url.trim().is_empty() {
                    None
                } else {
                    Some(api_url)
                };
            }
            if let Some(provider) = update.default_provider {
                config.default_provider = if provider.trim().is_empty() {
                    None
                } else {
                    Some(provider)
                };
            }
            if let Some(model) = update.default_model {
                config.default_model = if model.trim().is_empty() {
                    None
                } else {
                    Some(model)
                };
            }
            if let Some(temp) = update.default_temperature {
                config.default_temperature = temp;
            }
            config.save().await.map_err(|e| e.to_string())?;
            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "model settings saved to {}",
                    config.config_path.display()
                )],
            ))
        }

        "openhuman.update_memory_settings" => {
            let update: MemorySettingsUpdate = parse_params(params)?;
            let mut config = load_openhuman_config().await?;
            if let Some(backend) = update.backend {
                config.memory.backend = backend;
            }
            if let Some(auto_save) = update.auto_save {
                config.memory.auto_save = auto_save;
            }
            if let Some(provider) = update.embedding_provider {
                config.memory.embedding_provider = provider;
            }
            if let Some(model) = update.embedding_model {
                config.memory.embedding_model = model;
            }
            if let Some(dimensions) = update.embedding_dimensions {
                config.memory.embedding_dimensions = dimensions;
            }
            config.save().await.map_err(|e| e.to_string())?;
            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "memory settings saved to {}",
                    config.config_path.display()
                )],
            ))
        }

        "openhuman.update_gateway_settings" => {
            let update: GatewaySettingsUpdate = parse_params(params)?;
            let mut config = load_openhuman_config().await?;
            if let Some(host) = update.host {
                config.gateway.host = host;
            }
            if let Some(port) = update.port {
                config.gateway.port = port;
            }
            if let Some(require_pairing) = update.require_pairing {
                config.gateway.require_pairing = require_pairing;
            }
            if let Some(allow_public_bind) = update.allow_public_bind {
                config.gateway.allow_public_bind = allow_public_bind;
            }
            config.save().await.map_err(|e| e.to_string())?;
            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "gateway settings saved to {}",
                    config.config_path.display()
                )],
            ))
        }

        "openhuman.update_tunnel_settings" => {
            let tunnel: crate::openhuman::config::TunnelConfig = parse_params(params)?;
            let mut config = load_openhuman_config().await?;
            config.tunnel = tunnel;
            config.save().await.map_err(|e| e.to_string())?;
            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "tunnel settings saved to {}",
                    config.config_path.display()
                )],
            ))
        }

        "openhuman.update_runtime_settings" => {
            let update: RuntimeSettingsUpdate = parse_params(params)?;
            let mut config = load_openhuman_config().await?;
            if let Some(kind) = update.kind {
                config.runtime.kind = kind;
            }
            if let Some(reasoning_enabled) = update.reasoning_enabled {
                config.runtime.reasoning_enabled = Some(reasoning_enabled);
            }
            config.save().await.map_err(|e| e.to_string())?;
            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "runtime settings saved to {}",
                    config.config_path.display()
                )],
            ))
        }

        "openhuman.update_browser_settings" => {
            let update: BrowserSettingsUpdate = parse_params(params)?;
            let mut config = load_openhuman_config().await?;
            if let Some(enabled) = update.enabled {
                config.browser.enabled = enabled;
            }
            config.save().await.map_err(|e| e.to_string())?;
            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "browser settings saved to {}",
                    config.config_path.display()
                )],
            ))
        }

        "openhuman.get_runtime_flags" => {
            let flags = RuntimeFlags {
                browser_allow_all: env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"),
                log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
            };
            to_json_value(command_response(
                flags,
                vec!["runtime flags read".to_string()],
            ))
        }

        "openhuman.set_browser_allow_all" => {
            let p: SetBrowserAllowAllParams = parse_params(params)?;
            if p.enabled {
                std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL", "1");
            } else {
                std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
            }
            let flags = RuntimeFlags {
                browser_allow_all: env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"),
                log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
            };
            to_json_value(command_response(
                flags,
                vec!["browser allow-all flag updated".to_string()],
            ))
        }

        "openhuman.agent_chat" => {
            let p: AgentChatParams = parse_params(params)?;
            let mut config = load_openhuman_config().await?;
            if let Some(provider) = p.provider_override {
                config.default_provider = Some(provider);
            }
            if let Some(model) = p.model_override {
                config.default_model = Some(model);
            }
            if let Some(temp) = p.temperature {
                config.default_temperature = temp;
            }
            let mut agent =
                crate::openhuman::agent::Agent::from_config(&config).map_err(|e| e.to_string())?;
            let response = agent
                .run_single(&p.message)
                .await
                .map_err(|e| e.to_string())?;
            to_json_value(command_response(
                response,
                vec!["agent chat completed".to_string()],
            ))
        }

        "openhuman.local_ai_status" => {
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let status = service.status();
            if matches!(status.state.as_str(), "idle" | "degraded") {
                let service_clone = service.clone();
                let config_clone = config.clone();
                tokio::spawn(async move {
                    service_clone.bootstrap(&config_clone).await;
                });
            }
            to_json_value(command_response(
                service.status(),
                vec!["local ai status fetched".to_string()],
            ))
        }

        "openhuman.local_ai_download" => {
            let p: LocalAiDownloadParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let force = p.force.unwrap_or(false);
            if force {
                service.reset_to_idle(&config);
            }
            let service_clone = service.clone();
            let config_clone = config.clone();
            tokio::spawn(async move {
                service_clone.bootstrap(&config_clone).await;
            });
            to_json_value(command_response(
                service.status(),
                vec!["local ai bootstrap triggered".to_string()],
            ))
        }

        "openhuman.local_ai_summarize" => {
            let p: LocalAiSummarizeParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let status = service.status();
            if !matches!(status.state.as_str(), "ready") {
                service.bootstrap(&config).await;
            }
            let summary = service.summarize(&config, &p.text, p.max_tokens).await?;
            to_json_value(command_response(
                summary,
                vec!["local ai summarize completed".to_string()],
            ))
        }

        "openhuman.local_ai_suggest_questions" => {
            let p: LocalAiSuggestParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let status = service.status();
            if !matches!(status.state.as_str(), "ready") {
                service.bootstrap(&config).await;
            }
            let mut context = p.context.unwrap_or_default();
            if context.trim().is_empty() {
                if let Some(lines) = p.lines {
                    context = lines.join("\n");
                }
            }
            let suggestions: Vec<Suggestion> = service.suggest_questions(&config, &context).await?;
            to_json_value(command_response(
                suggestions,
                vec!["local ai suggestions generated".to_string()],
            ))
        }

        "openhuman.encrypt_secret" => {
            let p: EncryptSecretParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let store = secret_store_for_config(&config);
            let ciphertext = store.encrypt(&p.plaintext).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                ciphertext,
                vec!["secret encrypted".to_string()],
            ))
        }

        "openhuman.decrypt_secret" => {
            let p: DecryptSecretParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let store = secret_store_for_config(&config);
            let plaintext = store.decrypt(&p.ciphertext).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                plaintext,
                vec!["secret decrypted".to_string()],
            ))
        }

        "openhuman.doctor_report" => {
            let config = load_openhuman_config().await?;
            let report = doctor::run(&config).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                report,
                vec!["doctor report generated".to_string()],
            ))
        }

        "openhuman.doctor_models" => {
            let p: DoctorModelsParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let use_cache = p.use_cache.unwrap_or(true);
            let report = doctor::run_models(&config, p.provider_override.as_deref(), use_cache)
                .map_err(|e| e.to_string())?;
            to_json_value(command_response(
                report,
                vec!["model probes completed".to_string()],
            ))
        }

        "openhuman.list_integrations" => {
            let config = load_openhuman_config().await?;
            to_json_value(command_response(
                integrations::list_integrations(&config),
                vec!["integrations listed".to_string()],
            ))
        }

        "openhuman.get_integration_info" => {
            let p: IntegrationInfoParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let info =
                integrations::get_integration_info(&config, &p.name).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                info,
                vec![format!("integration loaded: {}", p.name)],
            ))
        }

        "openhuman.models_refresh" => {
            let p: ModelsRefreshParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let result = onboard::run_models_refresh(
                &config,
                p.provider_override.as_deref(),
                p.force.unwrap_or(false),
            )
            .map_err(|e| e.to_string())?;
            to_json_value(command_response(
                result,
                vec!["model refresh completed".to_string()],
            ))
        }

        "openhuman.migrate_openclaw" => {
            let p: MigrateOpenClawParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let source = p.source_workspace.map(std::path::PathBuf::from);
            let report =
                migration::migrate_openclaw_memory(&config, source, p.dry_run.unwrap_or(true))
                    .await
                    .map_err(|e| e.to_string())?;
            to_json_value(command_response(
                report,
                vec!["migration completed".to_string()],
            ))
        }

        "openhuman.hardware_discover" => to_json_value(command_response(
            hardware::discover_hardware(),
            vec!["hardware discovery complete".to_string()],
        )),

        "openhuman.hardware_introspect" => {
            let p: HardwareIntrospectParams = parse_params(params)?;
            let info = hardware::introspect_device(&p.path).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                info,
                vec![format!("introspected {}", p.path)],
            ))
        }

        "openhuman.service_install" => {
            let config = load_openhuman_config().await?;
            let status = service::install(&config).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                status,
                vec!["service install completed".to_string()],
            ))
        }

        "openhuman.service_start" => {
            let config = load_openhuman_config().await?;
            let status = service::start(&config).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                status,
                vec!["service start completed".to_string()],
            ))
        }

        "openhuman.service_stop" => {
            let config = load_openhuman_config().await?;
            let status = service::stop(&config).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                status,
                vec!["service stop completed".to_string()],
            ))
        }

        "openhuman.service_status" => {
            let config = load_openhuman_config().await?;
            let status = service::status(&config).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                status,
                vec!["service status fetched".to_string()],
            ))
        }

        "openhuman.service_uninstall" => {
            let config = load_openhuman_config().await?;
            let status = service::uninstall(&config).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                status,
                vec!["service uninstall completed".to_string()],
            ))
        }

        "openhuman.accessibility_status" => {
            let status = accessibility::global_engine().status().await;
            to_json_value(command_response(
                status,
                vec!["accessibility status fetched".to_string()],
            ))
        }

        "openhuman.accessibility_request_permissions" => {
            let permissions = accessibility::global_engine().request_permissions().await?;
            to_json_value(command_response(
                permissions,
                vec!["accessibility permissions requested".to_string()],
            ))
        }

        "openhuman.accessibility_start_session" => {
            let payload: StartSessionParams = parse_params(params)?;
            let session = accessibility::global_engine()
                .start_session(payload)
                .await?;
            to_json_value(command_response(
                session,
                vec!["accessibility session started".to_string()],
            ))
        }

        "openhuman.accessibility_stop_session" => {
            let payload: StopSessionParams = parse_params(params)?;
            let session = accessibility::global_engine()
                .stop_session(payload.reason)
                .await;
            to_json_value(command_response(
                session,
                vec!["accessibility session stopped".to_string()],
            ))
        }

        "openhuman.accessibility_capture_now" => {
            let result = accessibility::global_engine().capture_now().await?;
            to_json_value(command_response(
                result,
                vec!["accessibility manual capture requested".to_string()],
            ))
        }

        "openhuman.accessibility_input_action" => {
            let payload: InputActionParams = parse_params(params)?;
            let result = accessibility::global_engine().input_action(payload).await?;
            to_json_value(command_response(
                result,
                vec!["accessibility input action processed".to_string()],
            ))
        }

        "openhuman.accessibility_autocomplete_suggest" => {
            let payload: AutocompleteSuggestParams = parse_params(params)?;
            let result = accessibility::global_engine()
                .autocomplete_suggest(payload)
                .await?;
            to_json_value(command_response(
                result,
                vec!["accessibility autocomplete suggestions generated".to_string()],
            ))
        }

        "openhuman.accessibility_autocomplete_commit" => {
            let payload: AutocompleteCommitParams = parse_params(params)?;
            let result = accessibility::global_engine()
                .autocomplete_commit(payload)
                .await?;
            to_json_value(command_response(
                result,
                vec!["accessibility autocomplete suggestion committed".to_string()],
            ))
        }

        _ => Err(format!("unknown method: {method}")),
    }
}

pub async fn call_method(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    dispatch(
        AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        method,
        params,
    )
    .await
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

async fn root_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "name": "openhuman-core",
            "ok": true,
            "endpoints": {
                "health": "/health",
                "rpc": "/rpc"
            },
            "usage": {
                "jsonrpc": {
                    "version": "2.0",
                    "method": "core.ping",
                    "params": {}
                }
            }
        })),
    )
}

async fn not_found_handler() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "ok": false,
            "error": "not_found",
            "message": "Route not found. Try /, /health, or /rpc."
        })),
    )
}

fn core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

pub async fn run_server(port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or_else(core_port);
    let bind_addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/rpc", post(rpc_handler))
        .fallback(not_found_handler)
        .with_state(AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        });

    log::info!("[core] listening on http://{bind_addr}");

    tokio::spawn(async {
        match Config::load_or_init().await {
            Ok(config) if config.local_ai.enabled => {
                let service = local_ai::global(&config);
                service.bootstrap(&config).await;
            }
            Ok(_) => {}
            Err(err) => {
                log::warn!("[core] local-ai bootstrap skipped: {err}");
            }
        }
    });

    axum::serve(listener, app).await?;
    Ok(())
}

pub fn run_from_cli_args(args: &[String]) -> Result<()> {
    let mut port: Option<u16> = None;

    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "serve" => {}
            "--port" => {
                let value = args
                    .get(idx + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --port"))?;
                port = Some(value.parse::<u16>()?);
                idx += 1;
            }
            _ => {}
        }
        idx += 1;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_server(port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accessibility_status_rpc_returns_valid_schema() {
        let raw = call_method("openhuman.accessibility_status", json!({}))
            .await
            .expect("status rpc should return");
        let payload: CommandResponse<AccessibilityStatus> =
            serde_json::from_value(raw).expect("status payload should decode");

        assert!(!payload.logs.is_empty());
        assert!(!payload.result.config.capture_policy.is_empty());
    }

    #[tokio::test]
    async fn accessibility_start_session_requires_consent() {
        let err = call_method(
            "openhuman.accessibility_start_session",
            json!({
                "consent": false,
                "ttl_secs": 60
            }),
        )
        .await
        .expect_err("session start without consent should fail");

        assert!(err.contains("consent"));
    }

    #[tokio::test]
    async fn accessibility_input_action_rejects_invalid_envelope() {
        let err = call_method(
            "openhuman.accessibility_input_action",
            json!({
                "x": 10,
                "y": 20
            }),
        )
        .await
        .expect_err("missing action should fail envelope validation");

        assert!(err.contains("invalid params"));
    }
}
