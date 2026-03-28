use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::openhuman::autocomplete;
use crate::openhuman::config::Config;
use crate::openhuman::cron;
use crate::openhuman::health;
use crate::openhuman::local_ai::{
    self, LocalAiAssetsStatus, LocalAiEmbeddingResult, LocalAiSpeechResult, LocalAiTtsResult,
    Suggestion,
};
use crate::openhuman::security::{SecretStore, SecurityPolicy};
use crate::openhuman::tools::{ScreenshotTool, Tool};
use crate::openhuman::{
    doctor, hardware, integrations, migration, onboard, screen_intelligence, service,
};
use chrono::Utc;

const DEFAULT_CORE_RPC_URL: &str = "http://127.0.0.1:7788/rpc";
const DEFAULT_ONBOARDING_FLAG_NAME: &str = ".skip_onboarding";

pub use crate::openhuman::autocomplete::{
    AutocompleteAcceptParams, AutocompleteAcceptResult, AutocompleteCurrentParams,
    AutocompleteCurrentResult, AutocompleteDebugFocusResult, AutocompleteSetStyleParams,
    AutocompleteSetStyleResult, AutocompleteStartParams, AutocompleteStartResult,
    AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult,
};
pub use crate::openhuman::screen_intelligence::{
    AccessibilityStatus, AutocompleteCommitParams, AutocompleteCommitResult,
    AutocompleteSuggestParams, AutocompleteSuggestResult, CaptureImageRefResult, CaptureNowResult,
    InputActionParams, InputActionResult, PermissionRequestParams, PermissionState,
    PermissionStatus, SessionStatus, StartSessionParams, StopSessionParams, VisionFlushResult,
    VisionRecentResult,
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
pub struct ScreenIntelligenceSettingsUpdate {
    pub enabled: Option<bool>,
    pub capture_policy: Option<String>,
    pub policy_mode: Option<String>,
    pub baseline_fps: Option<f32>,
    pub vision_enabled: Option<bool>,
    pub autocomplete_enabled: Option<bool>,
    pub allowlist: Option<Vec<String>>,
    pub denylist: Option<Vec<String>>,
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
struct LocalAiPromptParams {
    prompt: String,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LocalAiSuggestParams {
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    lines: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct LocalAiVisionPromptParams {
    prompt: String,
    image_refs: Vec<String>,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LocalAiEmbedParams {
    inputs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeParams {
    audio_path: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeBytesParams {
    audio_bytes: Vec<u8>,
    extension: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiTtsParams {
    text: String,
    output_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccessibilityVisionRecentParams {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct LocalAiDownloadAssetParams {
    capability: String,
}

#[derive(Debug, Deserialize)]
struct CronJobIdParams {
    job_id: String,
}

#[derive(Debug, Deserialize)]
struct CronUpdateParams {
    job_id: String,
    patch: cron::CronJobPatch,
}

#[derive(Debug, Deserialize)]
struct CronRunsParams {
    job_id: String,
    limit: Option<usize>,
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

fn core_rpc_url() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL").unwrap_or_else(|_| DEFAULT_CORE_RPC_URL.to_string())
}

fn default_workspace_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openhuman")
        .join("workspace")
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

        "openhuman.update_screen_intelligence_settings" => {
            let update: ScreenIntelligenceSettingsUpdate = parse_params(params)?;
            let mut config = load_openhuman_config().await?;

            if let Some(enabled) = update.enabled {
                config.screen_intelligence.enabled = enabled;
            }
            if let Some(capture_policy) = update.capture_policy {
                config.screen_intelligence.capture_policy = capture_policy;
            }
            if let Some(policy_mode) = update.policy_mode {
                config.screen_intelligence.policy_mode = policy_mode;
            }
            if let Some(baseline_fps) = update.baseline_fps {
                config.screen_intelligence.baseline_fps = baseline_fps.clamp(0.2, 30.0);
            }
            if let Some(vision_enabled) = update.vision_enabled {
                config.screen_intelligence.vision_enabled = vision_enabled;
            }
            if let Some(autocomplete_enabled) = update.autocomplete_enabled {
                config.screen_intelligence.autocomplete_enabled = autocomplete_enabled;
            }
            if let Some(allowlist) = update.allowlist {
                config.screen_intelligence.allowlist = allowlist;
            }
            if let Some(denylist) = update.denylist {
                config.screen_intelligence.denylist = denylist;
            }

            config.save().await.map_err(|e| e.to_string())?;
            let _ = screen_intelligence::global_engine()
                .apply_config(config.screen_intelligence.clone())
                .await;

            let snapshot = snapshot_config(&config)?;
            to_json_value(command_response(
                snapshot,
                vec![format!(
                    "screen intelligence settings saved to {}",
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

        "openhuman.cron_list" => {
            let config = load_openhuman_config().await?;
            if !config.cron.enabled {
                return Err("cron is disabled by config (cron.enabled=false)".to_string());
            }
            let jobs = cron::list_jobs(&config).map_err(|e| e.to_string())?;
            to_json_value(command_response(jobs, vec!["cron jobs listed".to_string()]))
        }

        "openhuman.cron_update" => {
            let payload: CronUpdateParams = parse_params(params)?;
            if payload.job_id.trim().is_empty() {
                return Err("Missing 'job_id' parameter".to_string());
            }

            let config = load_openhuman_config().await?;
            if !config.cron.enabled {
                return Err("cron is disabled by config (cron.enabled=false)".to_string());
            }

            if let Some(command) = &payload.patch.command {
                let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
                if !security.is_command_allowed(command) {
                    return Err(format!("Command blocked by security policy: {command}"));
                }
            }

            let updated = cron::update_job(&config, payload.job_id.trim(), payload.patch)
                .map_err(|e| e.to_string())?;
            to_json_value(command_response(
                updated,
                vec![format!("cron job updated: {}", payload.job_id.trim())],
            ))
        }

        "openhuman.cron_remove" => {
            let payload: CronJobIdParams = parse_params(params)?;
            if payload.job_id.trim().is_empty() {
                return Err("Missing 'job_id' parameter".to_string());
            }

            let config = load_openhuman_config().await?;
            if !config.cron.enabled {
                return Err("cron is disabled by config (cron.enabled=false)".to_string());
            }

            cron::remove_job(&config, payload.job_id.trim()).map_err(|e| e.to_string())?;
            to_json_value(command_response(
                json!({ "job_id": payload.job_id.trim(), "removed": true }),
                vec![format!("cron job removed: {}", payload.job_id.trim())],
            ))
        }

        "openhuman.cron_run" => {
            let payload: CronJobIdParams = parse_params(params)?;
            if payload.job_id.trim().is_empty() {
                return Err("Missing 'job_id' parameter".to_string());
            }

            let config = load_openhuman_config().await?;
            if !config.cron.enabled {
                return Err("cron is disabled by config (cron.enabled=false)".to_string());
            }

            let job = cron::get_job(&config, payload.job_id.trim()).map_err(|e| e.to_string())?;
            let started_at = Utc::now();
            let (success, output) = cron::scheduler::execute_job_now(&config, &job).await;
            let finished_at = Utc::now();
            let duration_ms = (finished_at - started_at).num_milliseconds();
            let status = if success { "ok" } else { "error" };

            let _ = cron::record_run(
                &config,
                &job.id,
                started_at,
                finished_at,
                status,
                Some(&output),
                duration_ms,
            );
            let _ = cron::record_last_run(&config, &job.id, finished_at, success, &output);

            to_json_value(command_response(
                json!({
                    "job_id": job.id,
                    "status": status,
                    "duration_ms": duration_ms,
                    "output": output
                }),
                vec![format!("cron job run: {}", payload.job_id.trim())],
            ))
        }

        "openhuman.cron_runs" => {
            let payload: CronRunsParams = parse_params(params)?;
            if payload.job_id.trim().is_empty() {
                return Err("Missing 'job_id' parameter".to_string());
            }

            let config = load_openhuman_config().await?;
            if !config.cron.enabled {
                return Err("cron is disabled by config (cron.enabled=false)".to_string());
            }

            let limit = payload.limit.unwrap_or(20).max(1);
            let runs = cron::list_runs(&config, payload.job_id.trim(), limit)
                .map_err(|e| e.to_string())?;
            to_json_value(command_response(
                runs,
                vec![format!(
                    "cron run history loaded: {}",
                    payload.job_id.trim()
                )],
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
                if let Err(err) = service_clone.download_all_models(&config_clone).await {
                    service_clone.mark_degraded(err);
                }
            });
            to_json_value(command_response(
                service.status(),
                vec!["local ai full model download triggered".to_string()],
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

        "openhuman.local_ai_prompt" => {
            let p: LocalAiPromptParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let status = service.status();
            if !matches!(status.state.as_str(), "ready") {
                service.bootstrap(&config).await;
            }
            let output = service
                .prompt(
                    &config,
                    p.prompt.trim(),
                    p.max_tokens,
                    p.no_think.unwrap_or(true),
                )
                .await?;
            to_json_value(command_response(
                output,
                vec!["local ai prompt completed".to_string()],
            ))
        }

        "openhuman.local_ai_vision_prompt" => {
            let p: LocalAiVisionPromptParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let output = service
                .vision_prompt(&config, p.prompt.trim(), &p.image_refs, p.max_tokens)
                .await?;
            to_json_value(command_response(
                output,
                vec!["local ai vision prompt completed".to_string()],
            ))
        }

        "openhuman.local_ai_embed" => {
            let p: LocalAiEmbedParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let output: LocalAiEmbeddingResult = service.embed(&config, &p.inputs).await?;
            to_json_value(command_response(
                output,
                vec!["local ai embedding completed".to_string()],
            ))
        }

        "openhuman.local_ai_transcribe" => {
            let p: LocalAiTranscribeParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let output: LocalAiSpeechResult =
                service.transcribe(&config, p.audio_path.trim()).await?;
            to_json_value(command_response(
                output,
                vec!["local ai transcription completed".to_string()],
            ))
        }

        "openhuman.local_ai_transcribe_bytes" => {
            let p: LocalAiTranscribeBytesParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);

            let ext = p
                .extension
                .unwrap_or_else(|| "webm".to_string())
                .trim()
                .trim_start_matches('.')
                .to_ascii_lowercase();
            if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err("Invalid audio extension".to_string());
            }

            let voice_dir = std::env::temp_dir().join("openhuman_voice_input");
            tokio::fs::create_dir_all(&voice_dir)
                .await
                .map_err(|e| format!("Failed to create voice input directory: {e}"))?;

            let filename = format!(
                "voice-{}-{}.{}",
                Utc::now().timestamp_millis(),
                uuid::Uuid::new_v4(),
                ext
            );
            let file_path = voice_dir.join(filename);
            tokio::fs::write(&file_path, &p.audio_bytes)
                .await
                .map_err(|e| format!("Failed to write audio file: {e}"))?;

            let output = service
                .transcribe(&config, file_path.to_string_lossy().as_ref())
                .await;
            let _ = tokio::fs::remove_file(&file_path).await;

            let output = output?;
            to_json_value(command_response(
                output,
                vec!["local ai transcription completed".to_string()],
            ))
        }

        "openhuman.local_ai_tts" => {
            let p: LocalAiTtsParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let output: LocalAiTtsResult = service
                .tts(&config, p.text.trim(), p.output_path.as_deref())
                .await?;
            to_json_value(command_response(
                output,
                vec!["local ai tts completed".to_string()],
            ))
        }

        "openhuman.local_ai_assets_status" => {
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let output: LocalAiAssetsStatus = service.assets_status(&config).await?;
            to_json_value(command_response(
                output,
                vec!["local ai assets status fetched".to_string()],
            ))
        }

        "openhuman.local_ai_download_asset" => {
            let p: LocalAiDownloadAssetParams = parse_params(params)?;
            let config = load_openhuman_config().await?;
            let service = local_ai::global(&config);
            let output: LocalAiAssetsStatus =
                service.download_asset(&config, p.capability.trim()).await?;
            to_json_value(command_response(
                output,
                vec!["local ai asset download triggered".to_string()],
            ))
        }

        "ai.list_memory_files" => {
            #[derive(Debug, Deserialize)]
            struct ListMemoryFilesParams {
                relative_dir: Option<String>,
            }

            let payload: ListMemoryFilesParams = parse_params(params)?;
            let relative_dir = payload.relative_dir.unwrap_or_else(|| "memory".to_string());
            let files = crate::ai::sessions::ai_list_memory_files(relative_dir).await?;
            to_json_value(files)
        }

        "ai.read_memory_file" => {
            #[derive(Debug, Deserialize)]
            struct ReadMemoryFileParams {
                relative_path: String,
            }

            let payload: ReadMemoryFileParams = parse_params(params)?;
            let content = crate::ai::sessions::ai_read_memory_file(payload.relative_path).await?;
            to_json_value(content)
        }

        "ai.write_memory_file" => {
            #[derive(Debug, Deserialize)]
            struct WriteMemoryFileParams {
                relative_path: String,
                content: String,
            }

            let payload: WriteMemoryFileParams = parse_params(params)?;
            let wrote =
                crate::ai::sessions::ai_write_memory_file(payload.relative_path, payload.content)
                    .await?;
            to_json_value(wrote)
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

        "openhuman.workspace_onboarding_flag_exists" => {
            #[derive(Debug, Deserialize)]
            struct WorkspaceOnboardingFlagParams {
                flag_name: Option<String>,
            }

            let payload: WorkspaceOnboardingFlagParams = parse_params(params)?;
            let name = payload
                .flag_name
                .unwrap_or_else(|| DEFAULT_ONBOARDING_FLAG_NAME.to_string());
            let trimmed = name.trim();
            if trimmed.is_empty()
                || trimmed.contains('/')
                || trimmed.contains('\\')
                || trimmed.contains("..")
            {
                return Err("Invalid onboarding flag name".to_string());
            }

            let workspace_dir = match load_openhuman_config().await {
                Ok(cfg) => cfg.workspace_dir,
                Err(_) => default_workspace_dir(),
            };
            to_json_value(workspace_dir.join(trimmed).is_file())
        }

        "openhuman.agent_server_status" => {
            let payload = json!({
                "running": true,
                "url": core_rpc_url(),
            });
            to_json_value(command_response(
                payload,
                vec!["agent server status checked".to_string()],
            ))
        }

        "openhuman.accessibility_status" => {
            if let Ok(config) = load_openhuman_config().await {
                let _ = screen_intelligence::global_engine()
                    .apply_config(config.screen_intelligence.clone())
                    .await;
            }
            let status = screen_intelligence::global_engine().status().await;
            to_json_value(command_response(
                status,
                vec!["screen intelligence status fetched".to_string()],
            ))
        }

        "openhuman.accessibility_request_permissions" => {
            let permissions = screen_intelligence::global_engine()
                .request_permissions()
                .await?;
            to_json_value(command_response(
                permissions,
                vec!["accessibility permissions requested".to_string()],
            ))
        }

        "openhuman.accessibility_request_permission" => {
            let payload: PermissionRequestParams = parse_params(params)?;
            let permissions = screen_intelligence::global_engine()
                .request_permission(payload.permission)
                .await?;
            to_json_value(command_response(
                permissions,
                vec!["accessibility permission requested".to_string()],
            ))
        }

        "openhuman.accessibility_start_session" => {
            let _payload: StartSessionParams = parse_params(params)?;
            let session = screen_intelligence::global_engine().enable().await?;
            to_json_value(command_response(
                session,
                vec!["screen intelligence enabled".to_string()],
            ))
        }

        "openhuman.accessibility_stop_session" => {
            let payload: StopSessionParams = parse_params(params)?;
            let session = screen_intelligence::global_engine()
                .disable(payload.reason)
                .await;
            to_json_value(command_response(
                session,
                vec!["screen intelligence stopped".to_string()],
            ))
        }

        "openhuman.accessibility_capture_now" => {
            let result = screen_intelligence::global_engine().capture_now().await?;
            to_json_value(command_response(
                result,
                vec!["accessibility manual capture requested".to_string()],
            ))
        }

        "openhuman.accessibility_capture_image_ref" => {
            let result: CaptureImageRefResult = screen_intelligence::global_engine()
                .capture_image_ref_test()
                .await;
            to_json_value(command_response(
                result,
                vec!["accessibility direct image_ref capture requested".to_string()],
            ))
        }

        "openhuman.accessibility_input_action" => {
            let payload: InputActionParams = parse_params(params)?;
            let result = screen_intelligence::global_engine()
                .input_action(payload)
                .await?;
            to_json_value(command_response(
                result,
                vec!["screen intelligence input action processed".to_string()],
            ))
        }

        "openhuman.autocomplete_status" => {
            let result: AutocompleteStatus = autocomplete::global_engine().status().await;
            to_json_value(command_response(
                result,
                vec!["autocomplete status fetched".to_string()],
            ))
        }

        "openhuman.autocomplete_start" => {
            let payload: AutocompleteStartParams = parse_params(params)?;
            let result: AutocompleteStartResult =
                autocomplete::global_engine().start(payload).await?;
            to_json_value(command_response(
                result,
                vec!["autocomplete started".to_string()],
            ))
        }

        "openhuman.autocomplete_stop" => {
            let payload: Option<AutocompleteStopParams> = if params.is_null() {
                None
            } else {
                Some(parse_params(params)?)
            };
            let result: AutocompleteStopResult = autocomplete::global_engine().stop(payload).await;
            to_json_value(command_response(
                result,
                vec!["autocomplete stopped".to_string()],
            ))
        }

        "openhuman.autocomplete_current" => {
            let payload: Option<AutocompleteCurrentParams> = if params.is_null() {
                None
            } else {
                Some(parse_params(params)?)
            };
            let result: AutocompleteCurrentResult =
                autocomplete::global_engine().current(payload).await?;
            to_json_value(command_response(
                result,
                vec!["autocomplete suggestion fetched".to_string()],
            ))
        }

        "openhuman.autocomplete_debug_focus" => {
            let result: AutocompleteDebugFocusResult =
                autocomplete::global_engine().debug_focus().await?;
            to_json_value(command_response(
                result,
                vec!["autocomplete focus debug fetched".to_string()],
            ))
        }

        "openhuman.autocomplete_accept" => {
            let payload: AutocompleteAcceptParams = parse_params(params)?;
            let result: AutocompleteAcceptResult =
                autocomplete::global_engine().accept(payload).await?;
            to_json_value(command_response(
                result,
                vec!["autocomplete suggestion accepted".to_string()],
            ))
        }

        "openhuman.autocomplete_set_style" => {
            let payload: AutocompleteSetStyleParams = parse_params(params)?;
            let result: AutocompleteSetStyleResult =
                autocomplete::global_engine().set_style(payload).await?;
            to_json_value(command_response(
                result,
                vec!["autocomplete style settings updated".to_string()],
            ))
        }

        "openhuman.accessibility_vision_recent" => {
            let payload: AccessibilityVisionRecentParams = parse_params(params)?;
            let result: VisionRecentResult = screen_intelligence::global_engine()
                .vision_recent(payload.limit)
                .await;
            to_json_value(command_response(
                result,
                vec!["screen intelligence vision summaries fetched".to_string()],
            ))
        }

        "openhuman.accessibility_vision_flush" => {
            let result: VisionFlushResult =
                screen_intelligence::global_engine().vision_flush().await?;
            to_json_value(command_response(
                result,
                vec!["screen intelligence vision flush completed".to_string()],
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

#[derive(Debug, Parser)]
#[command(name = "openhuman-core")]
#[command(about = "OpenHuman core CLI")]
#[command(arg_required_else_help = true)]
struct CoreCli {
    #[command(subcommand)]
    command: CoreCommand,
}

#[derive(Debug, Subcommand)]
enum CoreCommand {
    /// Run JSON-RPC server
    #[command(alias = "serve")]
    Run {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Check core health
    Ping,
    /// Print core version
    Version,
    /// Get health snapshot
    Health,
    /// Get runtime flags
    RuntimeFlags,
    /// Get security policy info
    SecurityPolicy,
    /// Generic JSON-RPC style method call
    Call {
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// Generate shell completion scripts
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Settings style commands mirroring app settings sections
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// Accessibility automation commands
    Accessibility {
        #[command(subcommand)]
        command: AccessibilityCommand,
    },
    /// Standalone inline autocomplete commands
    Autocomplete {
        #[command(subcommand)]
        command: AutocompleteCommand,
    },
    /// Tool wrappers for local CLI testing
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    /// Legacy config operations
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SettingsCommand {
    Model {
        #[command(subcommand)]
        command: ModelSettingsCommand,
    },
    Memory {
        #[command(subcommand)]
        command: MemorySettingsCommand,
    },
    Gateway {
        #[command(subcommand)]
        command: GatewaySettingsCommand,
    },
    Tunnel {
        #[command(subcommand)]
        command: TunnelSettingsCommand,
    },
    Runtime {
        #[command(subcommand)]
        command: RuntimeSettingsCommand,
    },
    Browser {
        #[command(subcommand)]
        command: BrowserSettingsCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ModelSettingsCommand {
    Get,
    Set(ModelSetArgs),
}

#[derive(Debug, Args)]
struct ModelSetArgs {
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    api_url: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    temperature: Option<f64>,
}

#[derive(Debug, Subcommand)]
enum MemorySettingsCommand {
    Get,
    Set(MemorySetArgs),
}

#[derive(Debug, Args)]
struct MemorySetArgs {
    #[arg(long)]
    backend: Option<String>,
    #[arg(long)]
    auto_save: Option<bool>,
    #[arg(long)]
    embedding_provider: Option<String>,
    #[arg(long)]
    embedding_model: Option<String>,
    #[arg(long)]
    embedding_dimensions: Option<usize>,
}

#[derive(Debug, Subcommand)]
enum GatewaySettingsCommand {
    Get,
    Set(GatewaySetArgs),
}

#[derive(Debug, Args)]
struct GatewaySetArgs {
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    require_pairing: Option<bool>,
    #[arg(long)]
    allow_public_bind: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum TunnelSettingsCommand {
    Get,
    /// Replace tunnel settings with full JSON payload
    Set(TunnelSetArgs),
}

#[derive(Debug, Args)]
struct TunnelSetArgs {
    #[arg(long)]
    json: String,
}

#[derive(Debug, Subcommand)]
enum RuntimeSettingsCommand {
    Get,
    Set(RuntimeSetArgs),
}

#[derive(Debug, Args)]
struct RuntimeSetArgs {
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    reasoning_enabled: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum BrowserSettingsCommand {
    Get,
    Set(BrowserSetArgs),
}

#[derive(Debug, Subcommand)]
enum AccessibilityCommand {
    /// Read current accessibility automation status
    Status,
    /// Diagnose accessibility permission readiness with actionable fixes
    Doctor,
    /// Request all accessibility-related permissions
    RequestPermissions,
    /// Request a specific permission kind
    RequestPermission(RequestPermissionArgs),
    /// Start a bounded screen intelligence session
    StartSession(StartSessionCliArgs),
    /// Stop the active screen intelligence session
    StopSession(StopSessionCliArgs),
    /// Force an immediate capture sample
    CaptureNow,
    /// Directly trigger capture_screen_image_ref (no active session required)
    CaptureImageRef,
    /// Fetch recent vision summaries
    VisionRecent(VisionRecentCliArgs),
    /// Flush immediate vision summary from latest frame
    VisionFlush,
}

#[derive(Debug, Subcommand)]
enum AutocompleteCommand {
    Status,
    Start(AutocompleteStartCliArgs),
    Stop(AutocompleteStopCliArgs),
    Current(AutocompleteCurrentCliArgs),
    Accept(AutocompleteAcceptCliArgs),
    SetStyle(AutocompleteSetStyleCliArgs),
}

#[derive(Debug, Args)]
struct RequestPermissionArgs {
    /// One of: screen_recording, accessibility, input_monitoring
    #[arg(long)]
    permission: String,
}

#[derive(Debug, Args)]
struct StartSessionCliArgs {
    /// Explicit consent required to start
    #[arg(long, default_value_t = false)]
    consent: bool,
    /// Optional session TTL in seconds (bounded server-side)
    #[arg(long)]
    ttl_secs: Option<u64>,
    /// Optional override for screen monitoring
    #[arg(long)]
    screen_monitoring: Option<bool>,
    /// Optional override for device control
    #[arg(long)]
    device_control: Option<bool>,
    /// Optional override for predictive input
    #[arg(long)]
    predictive_input: Option<bool>,
}

#[derive(Debug, Args)]
struct StopSessionCliArgs {
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
struct VisionRecentCliArgs {
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct AutocompleteStartCliArgs {
    #[arg(long)]
    debounce_ms: Option<u64>,
}

#[derive(Debug, Args)]
struct AutocompleteStopCliArgs {
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
struct AutocompleteCurrentCliArgs {
    #[arg(long)]
    context: Option<String>,
}

#[derive(Debug, Args)]
struct AutocompleteAcceptCliArgs {
    #[arg(long)]
    suggestion: Option<String>,
}

#[derive(Debug, Args)]
struct AutocompleteSetStyleCliArgs {
    #[arg(long)]
    enabled: Option<bool>,
    #[arg(long)]
    debounce_ms: Option<u64>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[arg(long)]
    style_preset: Option<String>,
    #[arg(long)]
    style_instructions: Option<String>,
    #[arg(long)]
    style_example: Vec<String>,
    #[arg(long)]
    disabled_app: Vec<String>,
    #[arg(long)]
    accept_with_tab: Option<bool>,
}

#[derive(Debug, Args)]
struct BrowserSetArgs {
    #[arg(long)]
    enabled: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Get full config snapshot
    Get,
    /// Update model settings with a JSON object
    UpdateModel {
        #[arg(long)]
        json: String,
    },
    /// Update memory settings with a JSON object
    UpdateMemory {
        #[arg(long)]
        json: String,
    },
    /// Update gateway settings with a JSON object
    UpdateGateway {
        #[arg(long)]
        json: String,
    },
    /// Update runtime settings with a JSON object
    UpdateRuntime {
        #[arg(long)]
        json: String,
    },
    /// Update browser settings with a JSON object
    UpdateBrowser {
        #[arg(long)]
        json: String,
    },
    /// Replace tunnel settings with a JSON object
    UpdateTunnel {
        #[arg(long)]
        json: String,
    },
}

#[derive(Debug, Subcommand)]
enum ToolsCommand {
    /// List tool wrappers exposed by this CLI
    List,
    /// Capture a screenshot using the screenshot tool
    Screenshot(ToolsScreenshotArgs),
    /// Capture image ref directly from accessibility engine
    ScreenshotRef(ToolsScreenshotRefArgs),
    /// Generic wrapper for available tool commands
    Run(ToolsRunArgs),
}

#[derive(Debug, Args)]
struct ToolsScreenshotArgs {
    /// Optional filename saved under workspace
    #[arg(long)]
    filename: Option<String>,
    /// Optional region for macOS: selection | window
    #[arg(long)]
    region: Option<String>,
    /// Optional output file path (copies or writes PNG to this path)
    #[arg(long)]
    output: Option<PathBuf>,
    /// Include full data URL in JSON output
    #[arg(long, default_value_t = false)]
    print_data_url: bool,
}

#[derive(Debug, Args)]
struct ToolsScreenshotRefArgs {
    /// Optional output file path (writes PNG to this path)
    #[arg(long)]
    output: Option<PathBuf>,
    /// Include full data URL in JSON output
    #[arg(long, default_value_t = false)]
    print_data_url: bool,
}

#[derive(Debug, Args)]
struct ToolsRunArgs {
    /// Tool wrapper name: screenshot | screenshot-ref
    #[arg(long)]
    name: String,
    /// JSON arguments payload for selected wrapper
    #[arg(long, default_value = "{}")]
    args: String,
}

fn parse_json_arg(raw: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid JSON for --json/--params: {e}"))
}

fn ensure_non_empty_payload(payload: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    if payload.is_empty() {
        return Err(anyhow::anyhow!("no fields provided for set operation"));
    }
    Ok(())
}

fn extract_data_url(raw: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .starts_with("data:image/")
            .then(|| trimmed.to_string())
    })
}

fn extract_saved_path(raw: &str) -> Option<PathBuf> {
    const PREFIX: &str = "Screenshot saved to: ";
    raw.lines()
        .find_map(|line| line.strip_prefix(PREFIX).map(PathBuf::from))
}

fn decode_data_url_bytes(data_url: &str) -> Result<Vec<u8>, String> {
    let (meta, payload) = data_url
        .split_once(',')
        .ok_or_else(|| "invalid data URL: missing comma separator".to_string())?;
    if !meta.starts_with("data:image/") || !meta.ends_with(";base64") {
        return Err("invalid data URL: expected data:image/*;base64,...".to_string());
    }
    BASE64_STANDARD
        .decode(payload)
        .map_err(|e| format!("failed to decode base64 image payload: {e}"))
}

fn write_bytes_to_path(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create output directory: {e}"))?;
        }
    }
    std::fs::write(path, bytes).map_err(|e| format!("failed to write output file: {e}"))
}

async fn execute_tools_screenshot(args: ToolsScreenshotArgs) -> Result<serde_json::Value, String> {
    let config = load_openhuman_config().await?;
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let tool = ScreenshotTool::new(security);

    let mut payload = serde_json::Map::new();
    if let Some(filename) = args.filename {
        payload.insert("filename".to_string(), json!(filename));
    }
    if let Some(region) = args.region {
        payload.insert("region".to_string(), json!(region));
    }

    let tool_result = tool
        .execute(serde_json::Value::Object(payload))
        .await
        .map_err(|e| format!("screenshot tool failed to execute: {e}"))?;

    let mut logs = vec!["tools.screenshot executed".to_string()];

    if let Some(output_path) = args.output.as_ref() {
        if let Some(saved_path) = extract_saved_path(&tool_result.output) {
            std::fs::copy(&saved_path, output_path).map_err(|e| {
                format!(
                    "failed to copy screenshot from {} to {}: {e}",
                    saved_path.display(),
                    output_path.display()
                )
            })?;
            logs.push(format!("copied screenshot to {}", output_path.display()));
        } else if let Some(data_url) = extract_data_url(&tool_result.output) {
            let bytes = decode_data_url_bytes(&data_url)?;
            write_bytes_to_path(output_path, &bytes)?;
            logs.push(format!(
                "decoded data URL and wrote {} bytes to {}",
                bytes.len(),
                output_path.display()
            ));
        } else {
            return Err(
                "screenshot tool response did not contain a saved path or image data URL"
                    .to_string(),
            );
        }
    }

    let data_url = extract_data_url(&tool_result.output);
    let response = json!({
        "result": {
            "success": tool_result.success,
            "error": tool_result.error,
            "output_path": args.output.as_ref().map(|p| p.display().to_string()),
            "tool_output": tool_result.output,
            "data_url": if args.print_data_url { data_url } else { None::<String> },
        },
        "logs": logs
    });

    Ok(response)
}

async fn execute_tools_screenshot_ref(
    args: ToolsScreenshotRefArgs,
) -> Result<serde_json::Value, String> {
    let raw = call_method("openhuman.accessibility_capture_image_ref", json!({})).await?;
    let payload: CommandResponse<CaptureImageRefResult> =
        serde_json::from_value(raw).map_err(|e| {
            format!("failed to decode screen intelligence capture_image_ref response: {e}")
        })?;

    let mut logs = payload.logs;
    logs.push("tools.screenshot-ref executed".to_string());

    if let Some(output_path) = args.output.as_ref() {
        if let Some(data_url) = payload.result.image_ref.as_deref() {
            let bytes = decode_data_url_bytes(data_url)?;
            write_bytes_to_path(output_path, &bytes)?;
            logs.push(format!(
                "decoded image_ref and wrote {} bytes to {}",
                bytes.len(),
                output_path.display()
            ));
        } else {
            return Err(
                "screen intelligence capture_image_ref did not return image_ref".to_string(),
            );
        }
    }

    Ok(json!({
        "result": {
            "ok": payload.result.ok,
            "mime_type": payload.result.mime_type,
            "bytes_estimate": payload.result.bytes_estimate,
            "message": payload.result.message,
            "output_path": args.output.as_ref().map(|p| p.display().to_string()),
            "image_ref": if args.print_data_url { payload.result.image_ref } else { None::<String> },
        },
        "logs": logs
    }))
}

async fn get_config_snapshot() -> Result<CommandResponse<ConfigSnapshot>, String> {
    let value = call_method("openhuman.get_config", json!({})).await?;
    serde_json::from_value::<CommandResponse<ConfigSnapshot>>(value)
        .map_err(|e| format!("failed to decode config snapshot: {e}"))
}

fn settings_view_response(
    section: &'static str,
    snapshot: CommandResponse<ConfigSnapshot>,
) -> CommandResponse<serde_json::Value> {
    let cfg = &snapshot.result.config;
    let settings = match section {
        "model" => json!({
            "api_key": cfg.get("api_key"),
            "api_url": cfg.get("api_url"),
            "default_provider": cfg.get("default_provider"),
            "default_model": cfg.get("default_model"),
            "default_temperature": cfg.get("default_temperature"),
        }),
        "memory" => cfg
            .get("memory")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "gateway" => cfg
            .get("gateway")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "tunnel" => cfg
            .get("tunnel")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "runtime" => cfg
            .get("runtime")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "browser" => cfg
            .get("browser")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    };

    command_response(
        json!({
            "section": section,
            "settings": settings,
            "workspace_dir": snapshot.result.workspace_dir,
            "config_path": snapshot.result.config_path,
        }),
        snapshot.logs,
    )
}

async fn execute_core_cli(cli: CoreCli) -> Result<serde_json::Value, String> {
    match cli.command {
        CoreCommand::Run { port } => run_server(port)
            .await
            .map(|_| serde_json::Value::Null)
            .map_err(|e| format!("run failed: {e}")),
        CoreCommand::Ping => call_method("core.ping", json!({})).await,
        CoreCommand::Version => call_method("core.version", json!({})).await,
        CoreCommand::Health => call_method("openhuman.health_snapshot", json!({})).await,
        CoreCommand::RuntimeFlags => call_method("openhuman.get_runtime_flags", json!({})).await,
        CoreCommand::SecurityPolicy => {
            call_method("openhuman.security_policy_info", json!({})).await
        }
        CoreCommand::Call { method, params } => {
            call_method(&method, parse_json_arg(&params)?).await
        }
        CoreCommand::Completions { shell } => {
            let mut cmd = CoreCli::command();
            let bin_name = cmd.get_name().to_string();
            generate(shell, &mut cmd, bin_name, &mut io::stdout());
            Ok(serde_json::Value::Null)
        }
        CoreCommand::Settings { command } => match command {
            SettingsCommand::Model { command } => match command {
                ModelSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("model", snapshot))
                        .map_err(|e| e.to_string())
                }
                ModelSettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.api_key {
                        payload.insert("api_key".to_string(), json!(v));
                    }
                    if let Some(v) = args.api_url {
                        payload.insert("api_url".to_string(), json!(v));
                    }
                    if let Some(v) = args.provider {
                        payload.insert("default_provider".to_string(), json!(v));
                    }
                    if let Some(v) = args.model {
                        payload.insert("default_model".to_string(), json!(v));
                    }
                    if let Some(v) = args.temperature {
                        payload.insert("default_temperature".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    call_method(
                        "openhuman.update_model_settings",
                        serde_json::Value::Object(payload),
                    )
                    .await
                }
            },
            SettingsCommand::Memory { command } => match command {
                MemorySettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("memory", snapshot))
                        .map_err(|e| e.to_string())
                }
                MemorySettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.backend {
                        payload.insert("backend".to_string(), json!(v));
                    }
                    if let Some(v) = args.auto_save {
                        payload.insert("auto_save".to_string(), json!(v));
                    }
                    if let Some(v) = args.embedding_provider {
                        payload.insert("embedding_provider".to_string(), json!(v));
                    }
                    if let Some(v) = args.embedding_model {
                        payload.insert("embedding_model".to_string(), json!(v));
                    }
                    if let Some(v) = args.embedding_dimensions {
                        payload.insert("embedding_dimensions".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    call_method(
                        "openhuman.update_memory_settings",
                        serde_json::Value::Object(payload),
                    )
                    .await
                }
            },
            SettingsCommand::Gateway { command } => match command {
                GatewaySettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("gateway", snapshot))
                        .map_err(|e| e.to_string())
                }
                GatewaySettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.host {
                        payload.insert("host".to_string(), json!(v));
                    }
                    if let Some(v) = args.port {
                        payload.insert("port".to_string(), json!(v));
                    }
                    if let Some(v) = args.require_pairing {
                        payload.insert("require_pairing".to_string(), json!(v));
                    }
                    if let Some(v) = args.allow_public_bind {
                        payload.insert("allow_public_bind".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    call_method(
                        "openhuman.update_gateway_settings",
                        serde_json::Value::Object(payload),
                    )
                    .await
                }
            },
            SettingsCommand::Tunnel { command } => match command {
                TunnelSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("tunnel", snapshot))
                        .map_err(|e| e.to_string())
                }
                TunnelSettingsCommand::Set(args) => {
                    call_method(
                        "openhuman.update_tunnel_settings",
                        parse_json_arg(&args.json)?,
                    )
                    .await
                }
            },
            SettingsCommand::Runtime { command } => match command {
                RuntimeSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("runtime", snapshot))
                        .map_err(|e| e.to_string())
                }
                RuntimeSettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.kind {
                        payload.insert("kind".to_string(), json!(v));
                    }
                    if let Some(v) = args.reasoning_enabled {
                        payload.insert("reasoning_enabled".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    call_method(
                        "openhuman.update_runtime_settings",
                        serde_json::Value::Object(payload),
                    )
                    .await
                }
            },
            SettingsCommand::Browser { command } => match command {
                BrowserSettingsCommand::Get => {
                    let snapshot = get_config_snapshot().await?;
                    serde_json::to_value(settings_view_response("browser", snapshot))
                        .map_err(|e| e.to_string())
                }
                BrowserSettingsCommand::Set(args) => {
                    let mut payload = serde_json::Map::new();
                    if let Some(v) = args.enabled {
                        payload.insert("enabled".to_string(), json!(v));
                    }
                    ensure_non_empty_payload(&payload).map_err(|e| e.to_string())?;
                    call_method(
                        "openhuman.update_browser_settings",
                        serde_json::Value::Object(payload),
                    )
                    .await
                }
            },
        },
        CoreCommand::Accessibility { command } => match command {
            AccessibilityCommand::Status => {
                call_method("openhuman.accessibility_status", json!({})).await
            }
            AccessibilityCommand::Doctor => {
                let raw = call_method("openhuman.accessibility_status", json!({})).await?;
                let payload: CommandResponse<AccessibilityStatus> = serde_json::from_value(raw)
                    .map_err(|e| format!("failed to decode screen intelligence status: {e}"))?;
                let permissions = &payload.result.permissions;

                let screen_ready = permissions.screen_recording == PermissionState::Granted;
                let control_ready = permissions.accessibility == PermissionState::Granted;
                let monitoring_ready = permissions.input_monitoring == PermissionState::Granted;
                let overall_ready =
                    payload.result.platform_supported && screen_ready && control_ready;

                let mut recommendations: Vec<String> = Vec::new();
                if !payload.result.platform_supported {
                    recommendations.push(
                        "Accessibility automation is macOS-only in this build/runtime.".to_string(),
                    );
                }
                if permissions.screen_recording != PermissionState::Granted {
                    recommendations.push(
                        "Grant Screen Recording in System Settings -> Privacy & Security -> Screen Recording."
                            .to_string(),
                    );
                }
                if permissions.accessibility != PermissionState::Granted {
                    recommendations.push(
                        "Grant Accessibility in System Settings -> Privacy & Security -> Accessibility."
                            .to_string(),
                    );
                }
                if permissions.input_monitoring != PermissionState::Granted {
                    recommendations.push(
                        "Grant Input Monitoring in System Settings -> Privacy & Security -> Input Monitoring (optional but recommended)."
                            .to_string(),
                    );
                }
                if recommendations.is_empty() {
                    recommendations
                        .push("No action required. Accessibility automation is ready.".to_string());
                }

                Ok(json!({
                    "result": {
                        "summary": {
                            "overall_ready": overall_ready,
                            "platform_supported": payload.result.platform_supported,
                            "session_active": payload.result.session.active,
                            "screen_capture_ready": screen_ready,
                            "device_control_ready": control_ready,
                            "input_monitoring_ready": monitoring_ready
                        },
                        "permissions": permissions,
                        "features": payload.result.features,
                        "recommendations": recommendations
                    },
                    "logs": payload.logs
                }))
            }
            AccessibilityCommand::RequestPermissions => {
                call_method("openhuman.accessibility_request_permissions", json!({})).await
            }
            AccessibilityCommand::RequestPermission(args) => {
                call_method(
                    "openhuman.accessibility_request_permission",
                    json!({ "permission": args.permission }),
                )
                .await
            }
            AccessibilityCommand::StartSession(args) => {
                call_method(
                    "openhuman.accessibility_start_session",
                    json!({
                        "consent": args.consent,
                        "ttl_secs": args.ttl_secs,
                        "screen_monitoring": args.screen_monitoring,
                        "device_control": args.device_control,
                        "predictive_input": args.predictive_input,
                    }),
                )
                .await
            }
            AccessibilityCommand::StopSession(args) => {
                call_method(
                    "openhuman.accessibility_stop_session",
                    json!({ "reason": args.reason }),
                )
                .await
            }
            AccessibilityCommand::CaptureNow => {
                call_method("openhuman.accessibility_capture_now", json!({})).await
            }
            AccessibilityCommand::CaptureImageRef => {
                call_method("openhuman.accessibility_capture_image_ref", json!({})).await
            }
            AccessibilityCommand::VisionRecent(args) => {
                call_method(
                    "openhuman.accessibility_vision_recent",
                    json!({ "limit": args.limit }),
                )
                .await
            }
            AccessibilityCommand::VisionFlush => {
                call_method("openhuman.accessibility_vision_flush", json!({})).await
            }
        },
        CoreCommand::Autocomplete { command } => match command {
            AutocompleteCommand::Status => {
                call_method("openhuman.autocomplete_status", json!({})).await
            }
            AutocompleteCommand::Start(args) => {
                call_method(
                    "openhuman.autocomplete_start",
                    json!({ "debounce_ms": args.debounce_ms }),
                )
                .await
            }
            AutocompleteCommand::Stop(args) => {
                call_method(
                    "openhuman.autocomplete_stop",
                    json!({ "reason": args.reason }),
                )
                .await
            }
            AutocompleteCommand::Current(args) => {
                call_method(
                    "openhuman.autocomplete_current",
                    json!({ "context": args.context }),
                )
                .await
            }
            AutocompleteCommand::Accept(args) => {
                call_method(
                    "openhuman.autocomplete_accept",
                    json!({ "suggestion": args.suggestion }),
                )
                .await
            }
            AutocompleteCommand::SetStyle(args) => {
                let style_examples = (!args.style_example.is_empty()).then_some(args.style_example);
                let disabled_apps = (!args.disabled_app.is_empty()).then_some(args.disabled_app);
                call_method(
                    "openhuman.autocomplete_set_style",
                    json!({
                        "enabled": args.enabled,
                        "debounce_ms": args.debounce_ms,
                        "max_chars": args.max_chars,
                        "style_preset": args.style_preset,
                        "style_instructions": args.style_instructions,
                        "style_examples": style_examples,
                        "disabled_apps": disabled_apps,
                        "accept_with_tab": args.accept_with_tab,
                    }),
                )
                .await
            }
        },
        CoreCommand::Tools { command } => match command {
            ToolsCommand::List => Ok(json!({
                "result": {
                    "wrappers": [
                        {
                            "name": "screenshot",
                            "description": "Capture a screenshot with screenshot tool wrapper."
                        },
                        {
                            "name": "screenshot-ref",
                            "description": "Capture data URL from screen intelligence capture_image_ref."
                        }
                    ]
                },
                "logs": ["tools wrappers listed"]
            })),
            ToolsCommand::Screenshot(args) => execute_tools_screenshot(args).await,
            ToolsCommand::ScreenshotRef(args) => execute_tools_screenshot_ref(args).await,
            ToolsCommand::Run(args) => {
                let parsed = parse_json_arg(&args.args)?;
                match args.name.as_str() {
                    "screenshot" => {
                        let payload = parsed.as_object().cloned().unwrap_or_default();
                        let wrapped = ToolsScreenshotArgs {
                            filename: payload
                                .get("filename")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string),
                            region: payload
                                .get("region")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string),
                            output: payload
                                .get("output")
                                .and_then(serde_json::Value::as_str)
                                .map(PathBuf::from),
                            print_data_url: payload
                                .get("print_data_url")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                        };
                        execute_tools_screenshot(wrapped).await
                    }
                    "screenshot-ref" | "screenshot_ref" => {
                        let payload = parsed.as_object().cloned().unwrap_or_default();
                        let wrapped = ToolsScreenshotRefArgs {
                            output: payload
                                .get("output")
                                .and_then(serde_json::Value::as_str)
                                .map(PathBuf::from),
                            print_data_url: payload
                                .get("print_data_url")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                        };
                        execute_tools_screenshot_ref(wrapped).await
                    }
                    other => Err(format!(
                        "unsupported tool wrapper '{other}'. available: screenshot, screenshot-ref"
                    )),
                }
            }
        },
        CoreCommand::Config { command } => match command {
            ConfigCommand::Get => call_method("openhuman.get_config", json!({})).await,
            ConfigCommand::UpdateModel { json } => {
                call_method("openhuman.update_model_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateMemory { json } => {
                call_method("openhuman.update_memory_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateGateway { json } => {
                call_method("openhuman.update_gateway_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateRuntime { json } => {
                call_method("openhuman.update_runtime_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateBrowser { json } => {
                call_method("openhuman.update_browser_settings", parse_json_arg(&json)?).await
            }
            ConfigCommand::UpdateTunnel { json } => {
                call_method("openhuman.update_tunnel_settings", parse_json_arg(&json)?).await
            }
        },
    }
}

pub fn run_from_cli_args(args: &[String]) -> Result<()> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("openhuman-core".to_string());
    argv.extend(args.iter().cloned());
    let cli = CoreCli::try_parse_from(argv).map_err(|e| anyhow::anyhow!(e.render().to_string()))?;

    let thread_stack_size = std::env::var("OPENHUMAN_CORE_THREAD_STACK_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8 * 1024 * 1024);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(thread_stack_size)
        .enable_all()
        .build()?;
    let output = runtime
        .block_on(execute_core_cli(cli))
        .map_err(anyhow::Error::msg)?;
    if !output.is_null() {
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_else(|_| "null".to_string())
        );
    }
    Ok(())
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

    #[tokio::test]
    async fn autocomplete_status_rpc_returns_valid_schema() {
        let raw = call_method("openhuman.autocomplete_status", json!({}))
            .await
            .expect("autocomplete status rpc should return");
        let payload: CommandResponse<AutocompleteStatus> =
            serde_json::from_value(raw).expect("autocomplete status payload should decode");

        assert!(!payload.logs.is_empty());
    }
}
