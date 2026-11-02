use std::path::{Path, PathBuf};
use std::time::Instant;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use log::debug;

use crate::config::Config;
use crate::voice::{create_stt_provider, effective_stt_provider};
use tinyinference_local::models as model_ids;
use tinyinference_local::status::{LocalAiSpeechResult, LocalAiTtsResult};

use tinyinference_local::service::LocalAiService;

const LOG_PREFIX: &str = "[speech]";

pub async fn transcribe(
    service: &LocalAiService,
    config: &Config,
    audio_path: &str,
) -> Result<LocalAiSpeechResult, String> {
    transcribe_with_prompt(service, config, audio_path, None).await
}

/// Transcribe an audio file on disk.
///
/// **No longer local.** The bundled whisper.cpp engine (in-process
/// `whisper-rs` and the `whisper-cli` subprocess) was removed along with
/// its model downloader. Dispatch through the configured hosted STT engine
/// (`voice_server.stt_engine` or an explicit provider override), so every
/// dictation entry point honors the user's selected provider.
///
/// `initial_prompt` (the custom-dictionary vocabulary bias) is accepted for
/// signature compatibility and **ignored**: the backend transcription
/// endpoint exposes no prompt field. It is logged so a user wondering why
/// their dictionary stopped biasing results can find the reason.
pub async fn transcribe_with_prompt(
    service: &LocalAiService,
    config: &Config,
    audio_path: &str,
    initial_prompt: Option<&str>,
) -> Result<LocalAiSpeechResult, String> {
    let started = Instant::now();
    if let Some(prompt) = initial_prompt.filter(|p| !p.trim().is_empty()) {
        debug!(
            "{LOG_PREFIX} initial_prompt ({} chars) ignored — the hosted STT endpoint has no \
                 prompt/vocabulary-bias parameter",
            prompt.len()
        );
    }

    let provider_name = effective_stt_provider(config);
    debug!("{LOG_PREFIX} configured STT dispatch provider={provider_name} path={audio_path}");
    let provider = selected_stt_provider(config, &provider_name);
    let text =
        tinyinference_voice::transcription::transcribe_file(Path::new(audio_path), &provider)
            .await
            .map_err(|error| restore_host_file_error(error, audio_path))?;
    debug!(
        "{LOG_PREFIX} configured STT complete (provider={} elapsed_ms={})",
        provider_name,
        started.elapsed().as_millis()
    );
    service.mark_stt_ready();
    Ok(LocalAiSpeechResult {
        text,
        model_id: model_ids::effective_stt_model_id(config),
    })
}

fn restore_host_file_error(error: String, audio_path: &str) -> String {
    if let Some(reason) = error.strip_prefix("[voice-stt] failed to read audio file: ") {
        return format!("failed to read audio file {audio_path}: {reason}");
    }
    if error == "[voice-stt] audio file is empty" {
        return format!("audio file {audio_path} is empty");
    }
    error
}

/// Transcribe audio bytes with the host-selected STT provider.
pub async fn transcribe_bytes(
    service: &LocalAiService,
    config: &Config,
    audio: &[u8],
    extension: Option<&str>,
) -> Result<LocalAiSpeechResult, String> {
    let provider_name = effective_stt_provider(config);
    let provider = selected_stt_provider(config, &provider_name);
    let text =
        tinyinference_voice::transcription::transcribe_bytes(audio, extension, &provider).await?;
    service.mark_stt_ready();
    Ok(LocalAiSpeechResult {
        text,
        model_id: model_ids::effective_stt_model_id(config),
    })
}

fn selected_stt_provider<'a>(config: &'a Config, provider_name: &str) -> ConfiguredSttProvider<'a> {
    ConfiguredSttProvider {
        config,
        provider_name: provider_name.to_owned(),
    }
}

struct ConfiguredSttProvider<'a> {
    config: &'a Config,
    provider_name: String,
}

#[async_trait::async_trait]
impl tinyinference_voice::transcription::TranscriptionProvider for ConfiguredSttProvider<'_> {
    async fn transcribe(
        &self,
        audio: &[u8],
        mime_type: &str,
        file_name: &str,
    ) -> Result<String, String> {
        let provider = create_stt_provider(&self.provider_name, "", self.config)
            .await
            .map_err(|error| error.to_string())?;
        let audio_base64 = BASE64.encode(audio);
        let outcome = provider
            .transcribe(
                self.config,
                &audio_base64,
                Some(mime_type),
                Some(file_name),
                None,
            )
            .await?;
        Ok(outcome.value.text)
    }
}

pub async fn tts(
    service: &LocalAiService,
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<LocalAiTtsResult, String> {
    if !config.local_ai.runtime_enabled {
        return Err("local ai is disabled".to_string());
    }
    if text.trim().is_empty() {
        return Err("TTS text is required".to_string());
    }
    let runtime = crate::inference::local_runtime_config(config);
    let out_path = validated_tts_output_path(config, &runtime, output_path).await?;
    let piper_bin = tinyinference_local::service::paths::resolve_piper_binary_with_config(&runtime)
        .ok_or_else(|| "piper binary not found. Set PIPER_BIN or install piper.".to_string())?;
    let model_path = tinyinference_local::service::paths::resolve_tts_voice_path(&runtime)?;
    let parent = out_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "invalid output_path".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("failed to create TTS output directory: {e}"))?;

    tinyinference_voice::piper::synthesize_to_file(
        Path::new(&piper_bin),
        Path::new(&model_path),
        text,
        &out_path,
        None,
    )
    .await?;
    service.mark_tts_ready();
    Ok(LocalAiTtsResult {
        output_path: out_path.display().to_string(),
        voice_id: model_ids::effective_tts_voice_id(config),
    })
}

async fn validated_tts_output_path(
    config: &Config,
    runtime: &tinyinference_local::service::RuntimeConfig,
    requested_path: Option<&str>,
) -> Result<PathBuf, String> {
    let model_root = tinyinference_local::service::paths::config_root_dir(runtime)
        .join("models")
        .join("local-ai");
    if requested_path.is_some_and(|path| path.trim().is_empty()) {
        return Err("invalid output_path".to_string());
    }
    let requested = requested_path
        .map(PathBuf::from)
        .unwrap_or_else(|| model_root.join("tts-output.wav"));
    if requested.as_os_str().is_empty() {
        return Err("invalid output_path".to_string());
    }
    let candidate = if requested.is_absolute() {
        requested
    } else {
        config.workspace_dir.join(requested)
    };
    let candidate_string = candidate
        .to_str()
        .ok_or_else(|| "invalid output_path".to_string())?;
    let mut policy = crate::security::SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    );
    policy.trusted_roots.push(crate::security::TrustedRoot {
        path: config.workspace_dir.display().to_string(),
        access: crate::security::TrustedAccess::ReadWrite,
    });
    policy.trusted_roots.push(crate::security::TrustedRoot {
        path: model_root.display().to_string(),
        access: crate::security::TrustedAccess::ReadWrite,
    });
    if !policy.is_path_string_allowed(candidate_string) {
        return Err(format!("invalid output_path: {candidate_string}"));
    }
    let allowed_root = if candidate.starts_with(&config.workspace_dir) {
        &config.workspace_dir
    } else if candidate.starts_with(&model_root) {
        &model_root
    } else {
        return Err(
            "invalid output_path: path must be inside the workspace or local-ai models directory"
                .into(),
        );
    };
    let canonical_root = canonicalize_path_with_missing_suffix(allowed_root).await?;
    if crate::security::SecurityPolicy::is_always_forbidden_async(&canonical_root).await {
        return Err("invalid output_path: allowed root resolves to a protected path".into());
    }
    tokio::fs::create_dir_all(allowed_root)
        .await
        .map_err(|error| format!("could not prepare allowed TTS output root: {error}"))?;
    let resolved = match tokio::fs::symlink_metadata(&candidate).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("invalid output_path: symbolic links are not allowed".into());
        }
        Ok(_) => policy.validate_path(candidate_string).await,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            policy.validate_parent_path(candidate_string).await
        }
        Err(error) => Err(format!("could not inspect output_path: {error}")),
    }
    .map_err(|error| format!("invalid output_path: {error}"))?;

    let resolved_root = tokio::fs::canonicalize(allowed_root)
        .await
        .map_err(|error| format!("could not resolve allowed TTS output root: {error}"))?;
    if resolved.starts_with(&resolved_root) {
        return Ok(resolved);
    }
    Err(
        "invalid output_path: path must be inside the workspace or local-ai models directory"
            .into(),
    )
}

async fn canonicalize_path_with_missing_suffix(path: &std::path::Path) -> Result<PathBuf, String> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match tokio::fs::metadata(ancestor).await {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not resolve allowed TTS path root {}: {error}",
                    path.display()
                ));
            }
        }
        let name = ancestor.file_name().ok_or_else(|| {
            format!(
                "could not resolve allowed TTS path root: {}",
                path.display()
            )
        })?;
        missing.push(name.to_os_string());
        ancestor = ancestor.parent().ok_or_else(|| {
            format!(
                "could not resolve allowed TTS path root: {}",
                path.display()
            )
        })?;
    }
    let mut resolved = tokio::fs::canonicalize(ancestor)
        .await
        .map_err(|error| format!("could not resolve allowed TTS path root: {error}"))?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

#[cfg(test)]
#[path = "speech_tests.rs"]
mod tests;
