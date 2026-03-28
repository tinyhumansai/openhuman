use chrono::Utc;

use crate::core_server::helpers::{load_openhuman_config, parse_params};
use crate::core_server::types::{
    AgentChatParams, InvocationResult, LocalAiDownloadAssetParams,
    LocalAiDownloadParams, LocalAiEmbedParams, LocalAiPromptParams, LocalAiSuggestParams,
    LocalAiSummarizeParams, LocalAiTranscribeBytesParams, LocalAiTranscribeParams, LocalAiTtsParams,
    LocalAiVisionPromptParams,
};
use crate::openhuman::local_ai::{
    self, LocalAiAssetsStatus, LocalAiEmbeddingResult, LocalAiSpeechResult, LocalAiTtsResult,
    Suggestion,
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
                InvocationResult::with_logs(response, vec!["agent chat completed".to_string()])
            }
            .await,
        ),

        "openhuman.local_ai_status" => Some(
            async move {
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
                InvocationResult::with_logs(
                    service.status(),
                    vec!["local ai status fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_download" => Some(
            async move {
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
                InvocationResult::with_logs(
                    service.status(),
                    vec!["local ai full model download triggered".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_summarize" => Some(
            async move {
                let p: LocalAiSummarizeParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let service = local_ai::global(&config);
                let status = service.status();
                if !matches!(status.state.as_str(), "ready") {
                    service.bootstrap(&config).await;
                }
                let summary = service.summarize(&config, &p.text, p.max_tokens).await?;
                InvocationResult::with_logs(
                    summary,
                    vec!["local ai summarize completed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_suggest_questions" => Some(
            async move {
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
                let suggestions: Vec<Suggestion> =
                    service.suggest_questions(&config, &context).await?;
                InvocationResult::with_logs(
                    suggestions,
                    vec!["local ai suggestions generated".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_prompt" => Some(
            async move {
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
                InvocationResult::with_logs(output, vec!["local ai prompt completed".to_string()])
            }
            .await,
        ),

        "openhuman.local_ai_vision_prompt" => Some(
            async move {
                let p: LocalAiVisionPromptParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let service = local_ai::global(&config);
                let output = service
                    .vision_prompt(&config, p.prompt.trim(), &p.image_refs, p.max_tokens)
                    .await?;
                InvocationResult::with_logs(
                    output,
                    vec!["local ai vision prompt completed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_embed" => Some(
            async move {
                let p: LocalAiEmbedParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let service = local_ai::global(&config);
                let output: LocalAiEmbeddingResult = service.embed(&config, &p.inputs).await?;
                InvocationResult::with_logs(
                    output,
                    vec!["local ai embedding completed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_transcribe" => Some(
            async move {
                let p: LocalAiTranscribeParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let service = local_ai::global(&config);
                let output: LocalAiSpeechResult =
                    service.transcribe(&config, p.audio_path.trim()).await?;
                InvocationResult::with_logs(
                    output,
                    vec!["local ai transcription completed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_transcribe_bytes" => Some(
            async move {
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
                InvocationResult::with_logs(
                    output,
                    vec!["local ai transcription completed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_tts" => Some(
            async move {
                let p: LocalAiTtsParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let service = local_ai::global(&config);
                let output: LocalAiTtsResult = service
                    .tts(&config, p.text.trim(), p.output_path.as_deref())
                    .await?;
                InvocationResult::with_logs(output, vec!["local ai tts completed".to_string()])
            }
            .await,
        ),

        "openhuman.local_ai_assets_status" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let service = local_ai::global(&config);
                let output: LocalAiAssetsStatus = service.assets_status(&config).await?;
                InvocationResult::with_logs(
                    output,
                    vec!["local ai assets status fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.local_ai_download_asset" => Some(
            async move {
                let p: LocalAiDownloadAssetParams = parse_params(params)?;
                let config = load_openhuman_config().await?;
                let service = local_ai::global(&config);
                let output: LocalAiAssetsStatus =
                    service.download_asset(&config, p.capability.trim()).await?;
                InvocationResult::with_logs(
                    output,
                    vec!["local ai asset download triggered".to_string()],
                )
            }
            .await,
        ),

        _ => None,
    }
}
