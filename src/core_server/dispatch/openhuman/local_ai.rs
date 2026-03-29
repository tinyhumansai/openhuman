use crate::core_server::helpers::{
    load_openhuman_config, parse_params, rpc_invocation_from_outcome,
};
use crate::core_server::types::{
    AgentChatParams, InvocationResult, LocalAiDownloadAssetParams, LocalAiDownloadParams,
    LocalAiEmbedParams, LocalAiPromptParams, LocalAiSuggestParams, LocalAiSummarizeParams,
    LocalAiTranscribeBytesParams, LocalAiTranscribeParams, LocalAiTtsParams,
    LocalAiVisionPromptParams,
};

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.agent_chat" => Some(
            async move {
                let p: AgentChatParams = parse_params(params)?;
                let mut config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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

        "openhuman.local_ai_status" => Some(
            async move {
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::local_ai::rpc::local_ai_status(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_download" => Some(
            async move {
                let p: LocalAiDownloadParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::local_ai::rpc::local_ai_download(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::local_ai::rpc::local_ai_embed(&config, &p.inputs).await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_transcribe" => Some(
            async move {
                let p: LocalAiTranscribeParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
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
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::local_ai::rpc::local_ai_assets_status(&config).await?,
                )
            }
            .await,
        ),

        "openhuman.local_ai_download_asset" => Some(
            async move {
                let p: LocalAiDownloadAssetParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                rpc_invocation_from_outcome(
                    crate::openhuman::local_ai::rpc::local_ai_download_asset(
                        &config,
                        p.capability.trim(),
                    )
                    .await?,
                )
            }
            .await,
        ),

        _ => None,
    }
}
