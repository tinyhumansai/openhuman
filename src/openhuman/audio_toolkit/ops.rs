use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::Message;

use crate::openhuman::channels::email_channel::EmailChannel;
use crate::openhuman::config::Config;
use crate::openhuman::voice::{create_tts_provider, DEFAULT_PIPER_VOICE};
use crate::rpc::RpcOutcome;

use super::types::{
    AudioEmailDeliveryResult, AudioFormat, AudioGenerateRequest, AudioGeneratedArtifact,
    AudioToolkitGenerateAndEmailResult, EmailPodcastRequest,
};

const LOG_PREFIX: &str = "[audio_toolkit]";
const DEFAULT_OUTPUT_DIR: &str = "artifacts/audio";
const DEFAULT_CAPTURE_DIR: &str = "artifacts/email-capture";
const EMAIL_CAPTURE_ENV: &str = "OPENHUMAN_EMAIL_CAPTURE_DIR";

pub async fn generate_podcast(
    config: &Config,
    request: AudioGenerateRequest,
) -> Result<RpcOutcome<AudioGeneratedArtifact>, String> {
    let trimmed = request.text.trim();
    if trimmed.is_empty() {
        return Err("text is required".to_string());
    }

    let provider = effective_provider_name(config, request.provider.as_deref());
    let format = resolve_format(&provider, request.format)?;
    let voice = effective_voice(&provider, request.voice.as_deref());
    let output_rel_path = resolve_output_path(
        config.workspace_dir.as_path(),
        request.output_path.as_deref(),
        request.title.as_deref(),
        format,
    )?;

    log::debug!(
        "{LOG_PREFIX} generate provider={} format={:?} output_path={}",
        provider,
        format,
        output_rel_path.display()
    );

    let provider_impl = create_tts_provider(&provider, voice.as_deref().unwrap_or(""), config)
        .map_err(|e| e.to_string())?;
    let outcome = provider_impl
        .synthesize(config, trimmed, voice.as_deref())
        .await?;

    let bytes = decode_audio_payload(&outcome.value.audio_base64)?;
    enforce_audio_format(format, &outcome.value.audio_mime)?;
    let resolved = config.workspace_dir.join(&output_rel_path);
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    tokio::fs::write(&resolved, &bytes)
        .await
        .map_err(|e| format!("failed to write {}: {e}", resolved.display()))?;

    let result = AudioGeneratedArtifact {
        output_path: output_rel_path.to_string_lossy().to_string(),
        file_name: resolved
            .file_name()
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("podcast.{}", format.extension())),
        provider,
        voice,
        format,
        audio_mime: outcome.value.audio_mime,
        bytes_written: bytes.len(),
        chars_synthesized: trimmed.chars().count(),
    };

    Ok(RpcOutcome::single_log(
        result,
        "audio podcast synthesized to workspace file",
    ))
}

pub async fn email_podcast(
    config: &Config,
    request: EmailPodcastRequest,
) -> Result<RpcOutcome<AudioEmailDeliveryResult>, String> {
    let to = request.to.trim();
    let subject = request.subject.trim();
    let body = request.body.trim();
    let audio_path = request.audio_path.trim();

    if to.is_empty() {
        return Err("to is required".to_string());
    }
    if subject.is_empty() {
        return Err("subject is required".to_string());
    }
    if body.is_empty() {
        return Err("body is required".to_string());
    }
    if audio_path.is_empty() {
        return Err("audio_path is required".to_string());
    }

    let rel_audio_path = PathBuf::from(audio_path);
    if rel_audio_path.is_absolute() {
        return Err("audio_path must be workspace-relative".to_string());
    }
    if rel_audio_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("audio_path must not contain parent-directory traversal".to_string());
    }

    let resolved_audio_path = config.workspace_dir.join(&rel_audio_path);
    let audio_bytes = tokio::fs::read(&resolved_audio_path)
        .await
        .map_err(|e| format!("failed to read {}: {e}", resolved_audio_path.display()))?;
    let attachment_name = request
        .attachment_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            resolved_audio_path
                .file_name()
                .map(|v| v.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "podcast.mp3".to_string());
    let content_type = content_type_for_attachment(&attachment_name)?;

    let email = build_email_message(
        config,
        to,
        subject,
        body,
        &attachment_name,
        content_type,
        audio_bytes,
    )?;

    if let Some(capture_dir) = resolve_email_capture_dir(config) {
        let capture_rel =
            capture_email_message(config.workspace_dir.as_path(), &capture_dir, &email).await?;
        let result = AudioEmailDeliveryResult {
            to: to.to_string(),
            subject: subject.to_string(),
            attachment_name,
            mode: "capture".to_string(),
            capture_path: Some(capture_rel),
        };
        return Ok(RpcOutcome::single_log(
            result,
            "audio email captured to workspace file",
        ));
    }

    let email_cfg = config
        .channels_config
        .email
        .clone()
        .ok_or_else(|| "email channel is not configured".to_string())?;
    let channel = EmailChannel::new(email_cfg);
    channel
        .send_message(email)
        .map_err(|e| format!("failed to send email attachment: {e}"))?;

    Ok(RpcOutcome::single_log(
        AudioEmailDeliveryResult {
            to: to.to_string(),
            subject: subject.to_string(),
            attachment_name,
            mode: "smtp".to_string(),
            capture_path: None,
        },
        "audio email delivered over SMTP",
    ))
}

pub async fn generate_and_email_podcast(
    config: &Config,
    generate_request: AudioGenerateRequest,
    email_request: EmailPodcastRequest,
) -> Result<RpcOutcome<AudioToolkitGenerateAndEmailResult>, String> {
    let generated = generate_podcast(config, generate_request).await?;
    let email_request = EmailPodcastRequest {
        audio_path: generated.value.output_path.clone(),
        ..email_request
    };
    let emailed = email_podcast(config, email_request).await?;
    Ok(RpcOutcome::single_log(
        AudioToolkitGenerateAndEmailResult {
            audio: generated.value,
            email: emailed.value,
        },
        "audio podcast generated and emailed",
    ))
}

pub fn resolve_email_capture_dir(config: &Config) -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(EMAIL_CAPTURE_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    #[cfg(feature = "e2e-test-support")]
    {
        return Some(config.workspace_dir.join(DEFAULT_CAPTURE_DIR));
    }
    #[cfg(not(feature = "e2e-test-support"))]
    {
        let _ = config;
        None
    }
}

fn effective_provider_name(config: &Config, requested: Option<&str>) -> String {
    requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let configured = config.local_ai.tts_provider.trim();
            if configured.is_empty() {
                "cloud".to_string()
            } else {
                configured.to_string()
            }
        })
}

fn effective_voice(provider: &str, requested: Option<&str>) -> Option<String> {
    let explicit = requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if explicit.is_some() {
        return explicit;
    }
    if provider == "piper" {
        return Some(DEFAULT_PIPER_VOICE.to_string());
    }
    None
}

fn resolve_format(provider: &str, requested: Option<AudioFormat>) -> Result<AudioFormat, String> {
    let resolved = requested.unwrap_or_else(|| {
        if provider == "piper" {
            AudioFormat::Wav
        } else {
            AudioFormat::Mp3
        }
    });
    match (provider, resolved) {
        ("piper", AudioFormat::Mp3) => {
            Err("provider `piper` only supports wav output; use format=`wav` or provider=`cloud`".to_string())
        }
        ("cloud", AudioFormat::Wav) => {
            Err("provider `cloud` currently returns mp3 output only; use format=`mp3` or provider=`piper`".to_string())
        }
        _ => Ok(resolved),
    }
}

fn decode_audio_payload(audio_base64: &str) -> Result<Vec<u8>, String> {
    BASE64
        .decode(audio_base64.trim())
        .map_err(|e| format!("invalid audio_base64 payload: {e}"))
}

fn enforce_audio_format(format: AudioFormat, mime: &str) -> Result<(), String> {
    let normalized = mime.trim().to_ascii_lowercase();
    match format {
        AudioFormat::Mp3 if normalized == "audio/mpeg" => Ok(()),
        AudioFormat::Wav if normalized == "audio/wav" => Ok(()),
        _ => Err(format!(
            "provider returned mime `{mime}` but requested format was `{}`",
            format.extension()
        )),
    }
}

fn resolve_output_path(
    workspace_dir: &Path,
    requested: Option<&str>,
    title: Option<&str>,
    format: AudioFormat,
) -> Result<PathBuf, String> {
    let rel_path = match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => {
            let slug = slugify_title(title.unwrap_or("podcast"));
            PathBuf::from(DEFAULT_OUTPUT_DIR).join(format!(
                "{}-{}.{}",
                chrono::Utc::now().format("%Y%m%d-%H%M%S"),
                slug,
                format.extension()
            ))
        }
    };
    if rel_path.is_absolute() {
        return Err("output_path must be workspace-relative".to_string());
    }
    if rel_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("output_path must not contain parent-directory traversal".to_string());
    }
    let resolved = workspace_dir.join(&rel_path);
    if !resolved.starts_with(workspace_dir) {
        return Err("output_path escapes the workspace".to_string());
    }
    Ok(rel_path)
}

fn slugify_title(title: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "podcast".to_string()
    } else {
        trimmed.to_string()
    }
}

fn content_type_for_attachment(file_name: &str) -> Result<ContentType, String> {
    let mime = if file_name.to_ascii_lowercase().ends_with(".wav") {
        "audio/wav"
    } else {
        "audio/mpeg"
    };
    mime.parse()
        .map_err(|e| format!("invalid content type for attachment: {e}"))
}

fn build_email_message(
    config: &Config,
    to: &str,
    subject: &str,
    body: &str,
    attachment_name: &str,
    content_type: ContentType,
    attachment_bytes: Vec<u8>,
) -> Result<Message, String> {
    let from_address = config
        .channels_config
        .email
        .as_ref()
        .map(|cfg| cfg.from_address.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("openhuman@localhost.test");
    let from: Mailbox = from_address
        .parse()
        .map_err(|e| format!("invalid from address `{from_address}`: {e}"))?;
    let to_mailbox: Mailbox = to
        .parse()
        .map_err(|e| format!("invalid to address `{to}`: {e}"))?;
    let attachment =
        Attachment::new(attachment_name.to_string()).body(attachment_bytes, content_type);
    Message::builder()
        .from(from)
        .to(to_mailbox)
        .subject(subject)
        .multipart(
            MultiPart::mixed()
                .singlepart(SinglePart::plain(body.to_string()))
                .singlepart(attachment),
        )
        .map_err(|e| format!("failed to build email message: {e}"))
}

async fn capture_email_message(
    workspace_dir: &Path,
    capture_dir: &Path,
    email: &Message,
) -> Result<String, String> {
    let resolved_dir = if capture_dir.is_absolute() {
        capture_dir.to_path_buf()
    } else {
        workspace_dir.join(capture_dir)
    };
    tokio::fs::create_dir_all(&resolved_dir)
        .await
        .map_err(|e| format!("failed to create {}: {e}", resolved_dir.display()))?;
    let file_name = format!(
        "podcast-email-{}-{}.eml",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    );
    let resolved_path = resolved_dir.join(&file_name);
    tokio::fs::write(&resolved_path, email.formatted())
        .await
        .map_err(|e| format!("failed to write {}: {e}", resolved_path.display()))?;
    let relative = resolved_path
        .strip_prefix(workspace_dir)
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|_| resolved_path.to_string_lossy().to_string());
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        let mut config = Config::default();
        config.workspace_dir = std::env::temp_dir().join("openhuman-audio-toolkit-tests");
        config.local_ai.tts_provider = "cloud".to_string();
        config
    }

    #[test]
    fn resolve_format_defaults_to_mp3_for_cloud() {
        assert_eq!(resolve_format("cloud", None).unwrap(), AudioFormat::Mp3);
    }

    #[test]
    fn resolve_format_defaults_to_wav_for_piper() {
        assert_eq!(resolve_format("piper", None).unwrap(), AudioFormat::Wav);
    }

    #[test]
    fn resolve_format_rejects_mp3_for_piper() {
        let err = resolve_format("piper", Some(AudioFormat::Mp3)).unwrap_err();
        assert!(err.contains("only supports wav"));
    }

    #[test]
    fn slugify_title_collapses_noise() {
        assert_eq!(
            slugify_title(" Weekly update: Q2 / AI! "),
            "weekly-update-q2-ai"
        );
        assert_eq!(slugify_title("###"), "podcast");
    }

    #[test]
    fn resolve_output_path_rejects_parent_dir() {
        let err = resolve_output_path(
            Path::new("/tmp/workspace"),
            Some("../escape.mp3"),
            None,
            AudioFormat::Mp3,
        )
        .unwrap_err();
        assert!(err.contains("parent-directory traversal"));
    }

    #[test]
    fn build_email_message_includes_attachment_name() {
        let config = test_config();
        let message = build_email_message(
            &config,
            "listener@example.com",
            "Podcast",
            "Attached.",
            "briefing.mp3",
            "audio/mpeg".parse().unwrap(),
            vec![1, 2, 3],
        )
        .unwrap();
        let wire = String::from_utf8_lossy(&message.formatted()).to_string();
        assert!(wire.contains("Subject: Podcast"));
        assert!(wire.contains("filename=\"briefing.mp3\""));
        assert!(wire.contains("Content-Type: audio/mpeg"));
    }

    #[test]
    fn resolve_email_capture_dir_uses_workspace_when_e2e_feature_enabled() {
        let config = test_config();
        let capture = resolve_email_capture_dir(&config);
        #[cfg(feature = "e2e-test-support")]
        assert!(capture.unwrap().ends_with(DEFAULT_CAPTURE_DIR));
        #[cfg(not(feature = "e2e-test-support"))]
        assert!(capture.is_none());
    }

    #[test]
    fn effective_voice_defaults_for_piper_only() {
        assert_eq!(
            effective_voice("piper", None).as_deref(),
            Some(DEFAULT_PIPER_VOICE)
        );
        assert!(effective_voice("cloud", None).is_none());
    }

    #[test]
    fn enforce_audio_format_requires_matching_mime() {
        assert!(enforce_audio_format(AudioFormat::Mp3, "audio/mpeg").is_ok());
        assert!(enforce_audio_format(AudioFormat::Mp3, "audio/wav").is_err());
    }

    #[test]
    fn decode_audio_payload_rejects_bad_base64() {
        let err = decode_audio_payload("not-base64").unwrap_err();
        assert!(err.contains("invalid audio_base64"));
    }
}
