use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::screen_intelligence::{
    InputActionParams, PermissionRequestParams, StartSessionParams, StopSessionParams,
};
use crate::rpc::RpcOutcome;

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
struct MigrateOpenClawParams {
    source_workspace: Option<String>,
    dry_run: Option<bool>,
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

        "openhuman.security_policy_info" => Some(rpc_json(
            crate::openhuman::security::rpc::security_policy_info(),
        )),

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
