//! Operations over the user-run local runtime: status, summarize, prompt,
//! vision, transcription, and speech. OpenHuman never downloads models or
//! starts the runtime; it only probes and talks to the configured endpoint.

use chrono::Utc;

use crate::config::Config;
use crate::core::Outcome;
use crate::inference::host_runtime as local_ai;
use {
    tinyinference_local::status::LocalAiSpeechResult, tinyinference_local::status::LocalAiStatus,
    tinyinference_local::status::LocalAiTtsResult,
};

use super::turn_guards::enforce_user_prompt_or_reject;

/// Returns the current operational status of the local AI stack.
pub async fn local_ai_status(config: &Config) -> Result<Outcome<LocalAiStatus>, String> {
    let service = local_ai::global(config);
    let runtime = crate::inference::local_runtime_config(config);
    let status = service.status();
    // `unreachable` is re-probed on every status poll so a runtime the user
    // starts later is picked up. The probe is read-only (`GET /api/tags` or
    // `GET /v1/models`); it never spawns a runtime or pulls a model.
    if matches!(status.state.as_str(), "idle" | "degraded" | "unreachable") {
        tracing::debug!(
            state = %status.state,
            "[local_ai] status: scheduling endpoint probe"
        );
        let service_clone = service.clone();
        let config_clone = runtime.clone();
        tokio::spawn(async move {
            service_clone.bootstrap(&config_clone).await;
        });
    }
    // `LocalAiService` is a process-wide singleton whose cached `provider`
    // field was set at first init from whichever config it saw. After an
    // `inference_update_local_settings` call that swaps providers
    // (e.g. ollama → lm_studio) the cached value is stale, so we overlay
    // the current config's provider on the status snapshot before returning.
    let mut snapshot = service.status();
    snapshot.provider =
        tinyinference_local::provider::provider_from_name(&config.local_ai.provider)
            .as_str()
            .to_string();
    Ok(Outcome::single_log(snapshot, "local ai status fetched"))
}

/// Generates a summary of the provided text using local AI models.
pub async fn local_ai_summarize(
    config: &Config,
    text: &str,
    max_tokens: Option<u32>,
) -> Result<Outcome<String>, String> {
    enforce_user_prompt_or_reject(text.trim(), "local_ai.ops.local_ai_summarize")?;

    let service = local_ai::global(config);
    let runtime = crate::inference::local_runtime_config(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(&runtime).await;
    }
    let summary = service
        .summarize_interactive(&runtime, text, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(summary, "local ai summarize completed"))
}

/// Executes a raw prompt directly against the local AI model.
pub async fn local_ai_prompt(
    config: &Config,
    prompt: &str,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<Outcome<String>, String> {
    enforce_user_prompt_or_reject(prompt.trim(), "local_ai.ops.local_ai_prompt")?;

    let service = local_ai::global(config);
    let runtime = crate::inference::local_runtime_config(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(&runtime).await;
    }
    let output = service
        .prompt_interactive(
            &runtime,
            prompt.trim(),
            max_tokens,
            no_think.unwrap_or(true),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(output, "local ai prompt completed"))
}

/// Executes a multimodal (vision) prompt with associated images.
pub async fn local_ai_vision_prompt(
    config: &Config,
    prompt: &str,
    image_refs: &[String],
    max_tokens: Option<u32>,
) -> Result<Outcome<String>, String> {
    enforce_user_prompt_or_reject(prompt.trim(), "local_ai.ops.local_ai_vision_prompt")?;

    let service = local_ai::global(config);
    let runtime = crate::inference::local_runtime_config(config);
    let Some(_permit) = crate::platform::llm_gate::wait_for_capacity().await else {
        return Err("local AI vision inference is paused while signed out".to_string());
    };
    let output = service
        .vision_prompt(&runtime, prompt.trim(), image_refs, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(
        output,
        "local ai vision prompt completed",
    ))
}

/// Transcribes the audio file at the specified path.
pub async fn local_ai_transcribe(
    config: &Config,
    audio_path: &str,
) -> Result<Outcome<LocalAiSpeechResult>, String> {
    let service = local_ai::global(config);
    let output = local_ai::service::transcribe(&service, config, audio_path.trim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

/// Transcribes raw audio bytes by first saving them to a temporary file.
pub async fn local_ai_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
) -> Result<Outcome<LocalAiSpeechResult>, String> {
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

    let output =
        local_ai::service::transcribe(&service, config, file_path.to_string_lossy().as_ref()).await;
    let _ = tokio::fs::remove_file(&file_path).await;

    let output = output.map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

/// Performs text-to-speech synthesis and optionally saves the result to a file.
pub async fn local_ai_tts(
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<Outcome<LocalAiTtsResult>, String> {
    let service = local_ai::global(config);
    let output = local_ai::service::tts(&service, config, text.trim(), output_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(output, "local ai tts completed"))
}
