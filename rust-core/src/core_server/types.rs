use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse<T> {
    pub result: T,
    pub logs: Vec<String>,
}

pub fn command_response<T>(result: T, logs: Vec<String>) -> CommandResponse<T> {
    CommandResponse { result, logs }
}

/// Success payload from a core RPC handler before JSON-RPC wrapping.
#[derive(Debug, Clone)]
pub struct InvocationResult {
    pub value: serde_json::Value,
    pub logs: Vec<String>,
}

impl InvocationResult {
    pub fn ok<T: Serialize>(v: T) -> Result<Self, String> {
        Ok(Self {
            value: serde_json::to_value(v).map_err(|e| e.to_string())?,
            logs: vec![],
        })
    }

    pub fn with_logs<T: Serialize>(v: T, logs: Vec<String>) -> Result<Self, String> {
        Ok(Self {
            value: serde_json::to_value(v).map_err(|e| e.to_string())?,
            logs,
        })
    }

}

pub fn invocation_to_rpc_json(inv: InvocationResult) -> serde_json::Value {
    if inv.logs.is_empty() {
        inv.value
    } else {
        json!({ "result": inv.value, "logs": inv.logs })
    }
}

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RpcSuccess {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub result: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RpcFailure {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub error: RpcError,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct AppState {
    pub core_version: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStateResponse {
    pub is_authenticated: bool,
    pub user_id: Option<String>,
    pub user: Option<serde_json::Value>,
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileSummary {
    pub id: String,
    pub provider: String,
    pub profile_name: String,
    pub kind: String,
    pub account_id: Option<String>,
    pub workspace_id: Option<String>,
    pub metadata_keys: Vec<String>,
    pub updated_at: String,
    pub has_token: bool,
    pub has_token_set: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStoreSessionParams {
    pub token: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStoreProviderCredentialsParams {
    pub provider: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub fields: Option<serde_json::Value>,
    #[serde(default)]
    pub set_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthRemoveProviderCredentialsParams {
    pub provider: String,
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthListProviderCredentialsParams {
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SocketConnectParams {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SocketEmitParams {
    pub event: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct AgentChatParams {
    pub message: String,
    pub provider_override: Option<String>,
    pub model_override: Option<String>,
    pub temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct DoctorModelsParams {
    pub provider_override: Option<String>,
    pub use_cache: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct IntegrationInfoParams {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ModelsRefreshParams {
    pub provider_override: Option<String>,
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct MigrateOpenClawParams {
    pub source_workspace: Option<String>,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct HardwareIntrospectParams {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct EncryptSecretParams {
    pub plaintext: String,
}

#[derive(Debug, Deserialize)]
pub struct DecryptSecretParams {
    pub ciphertext: String,
}

#[derive(Debug, Deserialize)]
pub struct SetBrowserAllowAllParams {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiDownloadParams {
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiSummarizeParams {
    pub text: String,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiPromptParams {
    pub prompt: String,
    pub max_tokens: Option<u32>,
    pub no_think: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiSuggestParams {
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub lines: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiVisionPromptParams {
    pub prompt: String,
    pub image_refs: Vec<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiEmbedParams {
    pub inputs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiTranscribeParams {
    pub audio_path: String,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiTranscribeBytesParams {
    pub audio_bytes: Vec<u8>,
    pub extension: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiTtsParams {
    pub text: String,
    pub output_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AccessibilityVisionRecentParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct LocalAiDownloadAssetParams {
    pub capability: String,
}

#[derive(Debug, Deserialize)]
pub struct CronJobIdParams {
    pub job_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CronUpdateParams {
    pub job_id: String,
    pub patch: crate::openhuman::cron::CronJobPatch,
}

#[derive(Debug, Deserialize)]
pub struct CronRunsParams {
    pub job_id: String,
    pub limit: Option<usize>,
}
