//! JSON-RPC / CLI controller surface for the bundled local AI stack.

use chrono::Utc;
use once_cell::sync::Lazy;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::local_ai::{
    self, LocalAiAssetsStatus, LocalAiDownloadsProgress, LocalAiEmbeddingResult,
    LocalAiSpeechResult, LocalAiTtsResult, Suggestion,
};
use crate::openhuman::providers::{self, ProviderRuntimeOptions};
use crate::rpc::RpcOutcome;

static REPL_AGENT_SESSIONS: Lazy<Mutex<HashMap<String, Agent>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn agent_chat(
    config: &mut Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<String>, String> {
    if let Some(model) = model_override {
        config.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        config.default_temperature = temp;
    }
    let mut agent = Agent::from_config(config).map_err(|e| e.to_string())?;
    let response = agent.run_single(message).await.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(response, "agent chat completed"))
}

pub async fn agent_chat_simple(
    config: &Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<String>, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    let default_model = effective
        .default_model
        .clone()
        .unwrap_or_else(|| "neocortex-mk1".to_string());

    let options = ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: effective.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: effective.secrets.encrypt,
        reasoning_enabled: effective.runtime.reasoning_enabled,
    };

    let provider = providers::create_routed_provider_with_options(
        effective.api_key.as_deref(),
        effective.api_url.as_deref(),
        &effective.reliability,
        &effective.model_routes,
        default_model.as_str(),
        &options,
    )
    .map_err(|e| e.to_string())?;

    let response = provider
        .chat_with_system(
            None,
            message,
            default_model.as_str(),
            effective.default_temperature,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(
        response,
        "agent simple chat completed",
    ))
}

pub async fn agent_repl_session_start(
    config: &Config,
    session_id: Option<String>,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    let mut requested = session_id.unwrap_or_default();
    requested = requested.trim().to_string();
    let session_id = if requested.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        requested
    };

    let agent = Agent::from_config(&effective).map_err(|e| e.to_string())?;
    REPL_AGENT_SESSIONS
        .lock()
        .await
        .insert(session_id.clone(), agent);

    Ok(RpcOutcome::single_log(
        json!({ "session_id": session_id }),
        "agent repl session started",
    ))
}

pub async fn agent_repl_session_reset(
    session_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("session_id is required".to_string());
    }

    let mut sessions = REPL_AGENT_SESSIONS.lock().await;
    let reset = if let Some(agent) = sessions.get_mut(session_id) {
        agent.clear_history();
        true
    } else {
        false
    };

    Ok(RpcOutcome::single_log(
        json!({ "reset": reset }),
        "agent repl session reset",
    ))
}

pub async fn agent_repl_session_end(
    session_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("session_id is required".to_string());
    }

    let ended = REPL_AGENT_SESSIONS
        .lock()
        .await
        .remove(session_id)
        .is_some();
    Ok(RpcOutcome::single_log(
        json!({ "ended": ended }),
        "agent repl session ended",
    ))
}

pub async fn local_ai_status(
    config: &Config,
) -> Result<RpcOutcome<local_ai::LocalAiStatus>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if matches!(status.state.as_str(), "idle" | "degraded") {
        let service_clone = service.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            service_clone.bootstrap(&config_clone).await;
        });
    }
    Ok(RpcOutcome::single_log(
        service.status(),
        "local ai status fetched",
    ))
}

pub async fn local_ai_download(
    config: &Config,
    force: bool,
) -> Result<RpcOutcome<local_ai::LocalAiStatus>, String> {
    let service = local_ai::global(config);
    if force {
        service.reset_to_idle(config);
    }
    let service_clone = service.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        if let Err(err) = service_clone.download_all_models(&config_clone).await {
            service_clone.mark_degraded(err);
        }
    });
    Ok(RpcOutcome::single_log(
        service.status(),
        "local ai full model download triggered",
    ))
}

pub async fn local_ai_download_all_assets(
    config: &Config,
    force: bool,
) -> Result<RpcOutcome<LocalAiDownloadsProgress>, String> {
    let service = local_ai::global(config);
    if force {
        service.reset_to_idle(config);
    }
    let service_clone = service.clone();
    let config_clone = config.clone();
    tokio::spawn(async move {
        if let Err(err) = service_clone.download_all_models(&config_clone).await {
            service_clone.mark_degraded(err);
        }
    });
    let progress = service
        .downloads_progress(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        progress,
        "local ai full asset download triggered",
    ))
}

pub async fn local_ai_summarize(
    config: &Config,
    text: &str,
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let summary = service
        .summarize(config, text, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        summary,
        "local ai summarize completed",
    ))
}

pub async fn local_ai_suggest_questions(
    config: &Config,
    context: Option<String>,
    lines: Option<Vec<String>>,
) -> Result<RpcOutcome<Vec<Suggestion>>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let mut context = context.unwrap_or_default();
    if context.trim().is_empty() {
        if let Some(lines) = lines {
            context = lines.join("\n");
        }
    }
    let suggestions = service
        .suggest_questions(config, &context)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        suggestions,
        "local ai suggestions generated",
    ))
}

pub async fn local_ai_prompt(
    config: &Config,
    prompt: &str,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<RpcOutcome<String>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let output = service
        .prompt(config, prompt.trim(), max_tokens, no_think.unwrap_or(true))
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(output, "local ai prompt completed"))
}

pub async fn local_ai_vision_prompt(
    config: &Config,
    prompt: &str,
    image_refs: &[String],
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    let service = local_ai::global(config);
    let output = service
        .vision_prompt(config, prompt.trim(), image_refs, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai vision prompt completed",
    ))
}

pub async fn local_ai_embed(
    config: &Config,
    inputs: &[String],
) -> Result<RpcOutcome<LocalAiEmbeddingResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .embed(config, inputs)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai embedding completed",
    ))
}

pub async fn local_ai_transcribe(
    config: &Config,
    audio_path: &str,
) -> Result<RpcOutcome<LocalAiSpeechResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .transcribe(config, audio_path.trim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

pub async fn local_ai_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
) -> Result<RpcOutcome<LocalAiSpeechResult>, String> {
    let service = local_ai::global(config);

    let ext = extension
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
    tokio::fs::write(&file_path, audio_bytes)
        .await
        .map_err(|e| format!("Failed to write audio file: {e}"))?;

    let output = service
        .transcribe(config, file_path.to_string_lossy().as_ref())
        .await;
    let _ = tokio::fs::remove_file(&file_path).await;

    let output = output.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

pub async fn local_ai_tts(
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<RpcOutcome<LocalAiTtsResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .tts(config, text.trim(), output_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(output, "local ai tts completed"))
}

pub async fn local_ai_assets_status(
    config: &Config,
) -> Result<RpcOutcome<LocalAiAssetsStatus>, String> {
    let service = local_ai::global(config);
    let output = service
        .assets_status(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai assets status fetched",
    ))
}

pub async fn local_ai_downloads_progress(
    config: &Config,
) -> Result<RpcOutcome<LocalAiDownloadsProgress>, String> {
    let service = local_ai::global(config);
    let output = service
        .downloads_progress(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai downloads progress fetched",
    ))
}

pub async fn local_ai_download_asset(
    config: &Config,
    capability: &str,
) -> Result<RpcOutcome<LocalAiAssetsStatus>, String> {
    let service = local_ai::global(config);
    let output = service
        .download_asset(config, capability.trim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai asset download triggered",
    ))
}
