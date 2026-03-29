use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::openhuman::autocomplete::{
    AutocompleteAcceptParams, AutocompleteCurrentParams, AutocompleteSetStyleParams,
    AutocompleteStartParams, AutocompleteStopParams,
};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::screen_intelligence::{
    InputActionParams, PermissionRequestParams, StartSessionParams, StopSessionParams,
};
use crate::rpc::RpcOutcome;

const DEFAULT_ONBOARDING_FLAG_NAME: &str = ".skip_onboarding";

fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

fn rpc_json<T: Serialize>(outcome: RpcOutcome<T>) -> Result<serde_json::Value, String> {
    outcome.into_cli_compatible_json()
}

async fn load_config() -> Result<Config, String> {
    config_rpc::load_config_with_timeout().await
}

#[derive(Debug, Deserialize)]
struct ModelSettingsUpdate {
    api_key: Option<String>,
    api_url: Option<String>,
    default_model: Option<String>,
    default_temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct MemorySettingsUpdate {
    backend: Option<String>,
    auto_save: Option<bool>,
    embedding_provider: Option<String>,
    embedding_model: Option<String>,
    embedding_dimensions: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RuntimeSettingsUpdate {
    kind: Option<String>,
    reasoning_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BrowserSettingsUpdate {
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ScreenIntelligenceSettingsUpdate {
    enabled: Option<bool>,
    capture_policy: Option<String>,
    policy_mode: Option<String>,
    baseline_fps: Option<f32>,
    vision_enabled: Option<bool>,
    autocomplete_enabled: Option<bool>,
    allowlist: Option<Vec<String>>,
    denylist: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SetBrowserAllowAllParams {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct WorkspaceOnboardingFlagParams {
    flag_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CronJobIdParams {
    job_id: String,
}

#[derive(Debug, Deserialize)]
struct CronUpdateParams {
    job_id: String,
    patch: crate::openhuman::cron::CronJobPatch,
}

#[derive(Debug, Deserialize)]
struct CronRunsParams {
    job_id: String,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentReplSessionStartParams {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AgentReplSessionChatParams {
    session_id: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct AgentReplSessionControlParams {
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct DoctorModelsParams {
    use_cache: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct IntegrationInfoParams {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ModelsRefreshParams {
    force: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MigrateOpenClawParams {
    source_workspace: Option<String>,
    dry_run: Option<bool>,
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
struct LocalAiDownloadAssetParams {
    capability: String,
}

#[derive(Debug, Deserialize)]
struct AccessibilityVisionRecentParams {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthStoreSessionParams {
    token: String,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthConsumeLoginTokenParams {
    login_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthStoreProviderCredentialsParams {
    provider: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    fields: Option<serde_json::Value>,
    #[serde(default)]
    set_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthRemoveProviderCredentialsParams {
    provider: String,
    #[serde(default)]
    profile: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AuthListProviderCredentialsParams {
    #[serde(default)]
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthConnectParams {
    provider: String,
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    response_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthIntegrationTokensParams {
    integration_id: String,
    key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthOauthRevokeParams {
    integration_id: String,
}

#[derive(Debug, Deserialize)]
struct MemoryDocListParams {
    namespace: Option<String>,
}

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    match method {
        "memory.namespace.list" => Some(
            async move { rpc_json(crate::openhuman::memory::rpc::namespace_list().await?) }.await,
        ),

        "memory.doc.put" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::PutDocParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::doc_put(payload).await?)
            }
            .await,
        ),

        "memory.doc.list" => Some(
            async move {
                let payload: MemoryDocListParams = parse_params(params)?;
                let namespace_params = payload.namespace.map(|namespace| {
                    crate::openhuman::memory::rpc::NamespaceOnlyParams { namespace }
                });
                rpc_json(crate::openhuman::memory::rpc::doc_list(namespace_params).await?)
            }
            .await,
        ),

        "memory.doc.delete" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::DeleteDocParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::doc_delete(payload).await?)
            }
            .await,
        ),

        "memory.context.query" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::QueryNamespaceParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::context_query(payload).await?)
            }
            .await,
        ),

        "memory.context.recall" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::RecallNamespaceParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::context_recall(payload).await?)
            }
            .await,
        ),

        "memory.kv.set" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvSetParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_set(payload).await?)
            }
            .await,
        ),

        "memory.kv.get" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvGetDeleteParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_get(payload).await?)
            }
            .await,
        ),

        "memory.kv.delete" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvGetDeleteParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_delete(payload).await?)
            }
            .await,
        ),

        "memory.kv.list_namespace" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::NamespaceOnlyParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_list_namespace(payload).await?)
            }
            .await,
        ),

        "memory.graph.upsert" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::GraphUpsertParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::graph_upsert(payload).await?)
            }
            .await,
        ),

        "memory.graph.query" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::GraphQueryParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::graph_query(payload).await?)
            }
            .await,
        ),

        "openhuman.health_snapshot" => {
            Some(rpc_json(crate::openhuman::health::rpc::health_snapshot()))
        }

        "openhuman.security_policy_info" => Some(rpc_json(
            crate::openhuman::security::rpc::security_policy_info(),
        )),

        "openhuman.get_config" => {
            Some(async move { rpc_json(config_rpc::load_and_get_config_snapshot().await?) }.await)
        }

        "openhuman.update_model_settings" => Some(
            async move {
                let update: ModelSettingsUpdate = parse_params(params)?;
                let patch = config_rpc::ModelSettingsPatch {
                    api_key: update.api_key,
                    api_url: update.api_url,
                    default_model: update.default_model,
                    default_temperature: update.default_temperature,
                };
                rpc_json(config_rpc::load_and_apply_model_settings(patch).await?)
            }
            .await,
        ),

        "openhuman.update_memory_settings" => Some(
            async move {
                let update: MemorySettingsUpdate = parse_params(params)?;
                let patch = config_rpc::MemorySettingsPatch {
                    backend: update.backend,
                    auto_save: update.auto_save,
                    embedding_provider: update.embedding_provider,
                    embedding_model: update.embedding_model,
                    embedding_dimensions: update.embedding_dimensions,
                };
                rpc_json(config_rpc::load_and_apply_memory_settings(patch).await?)
            }
            .await,
        ),

        "openhuman.update_screen_intelligence_settings" => Some(
            async move {
                let update: ScreenIntelligenceSettingsUpdate = parse_params(params)?;
                let patch = config_rpc::ScreenIntelligenceSettingsPatch {
                    enabled: update.enabled,
                    capture_policy: update.capture_policy,
                    policy_mode: update.policy_mode,
                    baseline_fps: update.baseline_fps,
                    vision_enabled: update.vision_enabled,
                    autocomplete_enabled: update.autocomplete_enabled,
                    allowlist: update.allowlist,
                    denylist: update.denylist,
                };
                rpc_json(config_rpc::load_and_apply_screen_intelligence_settings(patch).await?)
            }
            .await,
        ),

        "openhuman.update_tunnel_settings" => Some(
            async move {
                let tunnel: crate::openhuman::config::TunnelConfig = parse_params(params)?;
                rpc_json(config_rpc::load_and_apply_tunnel_settings(tunnel).await?)
            }
            .await,
        ),

        "openhuman.update_runtime_settings" => Some(
            async move {
                let update: RuntimeSettingsUpdate = parse_params(params)?;
                let patch = config_rpc::RuntimeSettingsPatch {
                    kind: update.kind,
                    reasoning_enabled: update.reasoning_enabled,
                };
                rpc_json(config_rpc::load_and_apply_runtime_settings(patch).await?)
            }
            .await,
        ),

        "openhuman.update_browser_settings" => Some(
            async move {
                let update: BrowserSettingsUpdate = parse_params(params)?;
                let patch = config_rpc::BrowserSettingsPatch {
                    enabled: update.enabled,
                };
                rpc_json(config_rpc::load_and_apply_browser_settings(patch).await?)
            }
            .await,
        ),

        "openhuman.get_runtime_flags" => Some(rpc_json(config_rpc::get_runtime_flags())),

        "openhuman.set_browser_allow_all" => Some(
            async move {
                let payload: SetBrowserAllowAllParams = parse_params(params)?;
                rpc_json(config_rpc::set_browser_allow_all(payload.enabled))
            }
            .await,
        ),

        "openhuman.workspace_onboarding_flag_exists" => Some(
            async move {
                let payload: WorkspaceOnboardingFlagParams = parse_params(params)?;
                rpc_json(
                    config_rpc::workspace_onboarding_flag_resolve(
                        payload.flag_name,
                        DEFAULT_ONBOARDING_FLAG_NAME,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_server_status" => Some(rpc_json(config_rpc::agent_server_status())),

        "openhuman.cron_list" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::cron::rpc::cron_list(&config).await?)
            }
            .await,
        ),

        "openhuman.cron_update" => Some(
            async move {
                let payload: CronUpdateParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::cron::rpc::cron_update(
                        &config,
                        payload.job_id.trim(),
                        payload.patch,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.cron_remove" => Some(
            async move {
                let payload: CronJobIdParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::cron::rpc::cron_remove(&config, payload.job_id.trim())
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.cron_run" => Some(
            async move {
                let payload: CronJobIdParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::cron::rpc::cron_run(&config, payload.job_id.trim()).await?,
                )
            }
            .await,
        ),

        "openhuman.cron_runs" => Some(
            async move {
                let payload: CronRunsParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::cron::rpc::cron_runs(
                        &config,
                        payload.job_id.trim(),
                        payload.limit,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_chat" => Some(
            async move {
                let p: AgentChatParams = parse_params(params)?;
                let mut config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::agent_chat(
                        &mut config,
                        &p.message,
                        p.model_override,
                        p.temperature,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_chat_simple" => Some(
            async move {
                let p: AgentChatParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::agent_chat_simple(
                        &config,
                        &p.message,
                        p.model_override,
                        p.temperature,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_repl_session_start" => Some(
            async move {
                let p: AgentReplSessionStartParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::agent_repl_session_start(
                        &config,
                        p.session_id,
                        p.model_override,
                        p.temperature,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_repl_session_chat" => Some(
            async move {
                let p: AgentReplSessionChatParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::agent_repl_session_chat(
                        p.session_id.trim(),
                        &p.message,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_repl_session_reset" => Some(
            async move {
                let p: AgentReplSessionControlParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::agent_repl_session_reset(p.session_id.trim())
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.agent_repl_session_end" => Some(
            async move {
                let p: AgentReplSessionControlParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::agent_repl_session_end(p.session_id.trim())
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_status" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::local_ai::rpc::local_ai_status(&config).await?)
            }
            .await,
        ),

        "openhuman.local_ai_download" => Some(
            async move {
                let p: LocalAiDownloadParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_download(
                        &config,
                        p.force.unwrap_or(false),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_download_all_assets" => Some(
            async move {
                let p: LocalAiDownloadParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_download_all_assets(
                        &config,
                        p.force.unwrap_or(false),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_summarize" => Some(
            async move {
                let p: LocalAiSummarizeParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_summarize(
                        &config,
                        &p.text,
                        p.max_tokens,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_suggest_questions" => Some(
            async move {
                let p: LocalAiSuggestParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_suggest_questions(
                        &config, p.context, p.lines,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_prompt" => Some(
            async move {
                let p: LocalAiPromptParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_prompt(
                        &config,
                        &p.prompt,
                        p.max_tokens,
                        p.no_think,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_vision_prompt" => Some(
            async move {
                let p: LocalAiVisionPromptParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_vision_prompt(
                        &config,
                        &p.prompt,
                        &p.image_refs,
                        p.max_tokens,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_embed" => Some(
            async move {
                let p: LocalAiEmbedParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(crate::openhuman::local_ai::rpc::local_ai_embed(&config, &p.inputs).await?)
            }
            .await,
        ),

        "openhuman.local_ai_transcribe" => Some(
            async move {
                let p: LocalAiTranscribeParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_transcribe(
                        &config,
                        p.audio_path.trim(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_transcribe_bytes" => Some(
            async move {
                let p: LocalAiTranscribeBytesParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_transcribe_bytes(
                        &config,
                        &p.audio_bytes,
                        p.extension,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_tts" => Some(
            async move {
                let p: LocalAiTtsParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_tts(
                        &config,
                        &p.text,
                        p.output_path.as_deref(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_assets_status" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::local_ai::rpc::local_ai_assets_status(&config).await?)
            }
            .await,
        ),

        "openhuman.local_ai_downloads_progress" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_downloads_progress(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_download_asset" => Some(
            async move {
                let p: LocalAiDownloadAssetParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::local_ai::rpc::local_ai_download_asset(
                        &config,
                        p.capability.trim(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_status" => Some(
            async move {
                rpc_json(crate::openhuman::screen_intelligence::rpc::accessibility_status().await?)
            }
            .await,
        ),

        "openhuman.accessibility_request_permissions" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_request_permissions()
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_request_permission" => Some(
            async move {
                let payload: PermissionRequestParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_request_permission(
                        payload,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_start_session" => Some(
            async move {
                let payload: StartSessionParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_start_session(
                        payload,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_stop_session" => Some(
            async move {
                let payload: StopSessionParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_stop_session(payload)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_capture_now" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_capture_now().await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_capture_image_ref" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_capture_image_ref()
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_input_action" => Some(
            async move {
                let payload: InputActionParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_input_action(payload)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.autocomplete_status" => {
            Some(
                async move {
                    rpc_json(crate::openhuman::autocomplete::rpc::autocomplete_status().await?)
                }
                .await,
            )
        }

        "openhuman.autocomplete_start" => Some(
            async move {
                let payload: AutocompleteStartParams = parse_params(params)?;
                rpc_json(crate::openhuman::autocomplete::rpc::autocomplete_start(payload).await?)
            }
            .await,
        ),

        "openhuman.autocomplete_stop" => Some(
            async move {
                let payload: Option<AutocompleteStopParams> = if params.is_null() {
                    None
                } else {
                    Some(parse_params(params)?)
                };
                rpc_json(crate::openhuman::autocomplete::rpc::autocomplete_stop(payload).await?)
            }
            .await,
        ),

        "openhuman.autocomplete_current" => Some(
            async move {
                let payload: Option<AutocompleteCurrentParams> = if params.is_null() {
                    None
                } else {
                    Some(parse_params(params)?)
                };
                rpc_json(crate::openhuman::autocomplete::rpc::autocomplete_current(payload).await?)
            }
            .await,
        ),

        "openhuman.autocomplete_debug_focus" => Some(
            async move {
                rpc_json(crate::openhuman::autocomplete::rpc::autocomplete_debug_focus().await?)
            }
            .await,
        ),

        "openhuman.autocomplete_accept" => Some(
            async move {
                let payload: AutocompleteAcceptParams = parse_params(params)?;
                rpc_json(crate::openhuman::autocomplete::rpc::autocomplete_accept(payload).await?)
            }
            .await,
        ),

        "openhuman.autocomplete_set_style" => Some(
            async move {
                let payload: AutocompleteSetStyleParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::autocomplete::rpc::autocomplete_set_style(payload).await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_vision_recent" => Some(
            async move {
                let payload: AccessibilityVisionRecentParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_vision_recent(
                        payload.limit,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.accessibility_vision_flush" => Some(
            async move {
                rpc_json(
                    crate::openhuman::screen_intelligence::rpc::accessibility_vision_flush()
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.encrypt_secret" => Some(
            async move {
                let p: EncryptSecretParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::credentials::rpc::encrypt_secret(&config, &p.plaintext)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.decrypt_secret" => Some(
            async move {
                let p: DecryptSecretParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::credentials::rpc::decrypt_secret(&config, &p.ciphertext)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.doctor_report" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::doctor::rpc::doctor_report(&config).await?)
            }
            .await,
        ),

        "openhuman.doctor_models" => Some(
            async move {
                let p: DoctorModelsParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::doctor::rpc::doctor_models(
                        &config,
                        p.use_cache.unwrap_or(true),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.list_integrations" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::integrations::rpc::list_integrations(&config).await?)
            }
            .await,
        ),

        "openhuman.get_integration_info" => Some(
            async move {
                let p: IntegrationInfoParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::integrations::rpc::get_integration_info(&config, &p.name)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.models_refresh" => Some(
            async move {
                let p: ModelsRefreshParams = parse_params(params)?;
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::model_catalog::rpc::models_refresh(
                        &config,
                        p.force.unwrap_or(false),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.migrate_openclaw" => Some(
            async move {
                let p: MigrateOpenClawParams = parse_params(params)?;
                let config = load_config().await?;
                let source = p.source_workspace.map(std::path::PathBuf::from);
                rpc_json(
                    crate::openhuman::migration::rpc::migrate_openclaw(
                        &config,
                        source,
                        p.dry_run.unwrap_or(true),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.service_install" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::service::rpc::service_install(&config).await?)
            }
            .await,
        ),

        "openhuman.service_start" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::service::rpc::service_start(&config).await?)
            }
            .await,
        ),

        "openhuman.service_stop" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::service::rpc::service_stop(&config).await?)
            }
            .await,
        ),

        "openhuman.service_status" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::service::rpc::service_status(&config).await?)
            }
            .await,
        ),

        "openhuman.service_uninstall" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::service::rpc::service_uninstall(&config).await?)
            }
            .await,
        ),

        "openhuman.auth.store_session" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthStoreSessionParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::credentials::rpc::store_session(
                        &config,
                        &payload.token,
                        payload.user_id,
                        payload.user,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.clear_session" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::credentials::rpc::clear_session(&config).await?)
            }
            .await,
        ),

        "openhuman.auth.get_state" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(crate::openhuman::credentials::rpc::auth_get_state(&config).await?)
            }
            .await,
        ),

        "openhuman.auth.get_session_token" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::credentials::rpc::auth_get_session_token_json(&config)
                        .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.consume_login_token" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthConsumeLoginTokenParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::credentials::rpc::consume_login_token(
                        &config,
                        payload.login_token.trim(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.store_provider_credentials" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthStoreProviderCredentialsParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::credentials::rpc::store_provider_credentials(
                        &config,
                        &payload.provider,
                        payload.profile.as_deref(),
                        payload.token,
                        payload.fields,
                        payload.set_active,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.remove_provider_credentials" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthRemoveProviderCredentialsParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::credentials::rpc::remove_provider_credentials(
                        &config,
                        &payload.provider,
                        payload.profile.as_deref(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.list_provider_credentials" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthListProviderCredentialsParams = if params.is_null() {
                    AuthListProviderCredentialsParams::default()
                } else {
                    parse_params(params)?
                };
                let provider_filter = payload
                    .provider
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string);

                rpc_json(
                    crate::openhuman::credentials::rpc::list_provider_credentials(
                        &config,
                        provider_filter,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.oauth_connect" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthOauthConnectParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::credentials::rpc::oauth_connect(
                        &config,
                        payload.provider.trim(),
                        payload.skill_id.as_deref().map(str::trim),
                        payload.response_type.as_deref().map(str::trim),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.oauth_list_integrations" => Some(
            async move {
                let config = load_config().await?;
                rpc_json(
                    crate::openhuman::credentials::rpc::oauth_list_integrations(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.auth.oauth_fetch_integration_tokens" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthOauthIntegrationTokensParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::credentials::rpc::oauth_fetch_integration_tokens(
                        &config,
                        payload.integration_id.trim(),
                        payload.key.trim(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.auth.oauth_revoke_integration" => Some(
            async move {
                let config = load_config().await?;
                let payload: AuthOauthRevokeParams = parse_params(params)?;
                rpc_json(
                    crate::openhuman::credentials::rpc::oauth_revoke_integration(
                        &config,
                        payload.integration_id.trim(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        "openhuman.socket.connect" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        "openhuman.socket.disconnect" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        "openhuman.socket.state" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        "openhuman.socket.emit" => Some(Err(
            "native skill runtime and socket manager are not available in this build".to_string(),
        )),

        _ => None,
    }
}
