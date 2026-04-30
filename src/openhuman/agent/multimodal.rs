use crate::openhuman::config::{build_runtime_proxy_client_with_timeouts, MultimodalConfig};
use crate::openhuman::providers::ChatMessage;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::Client;
use std::path::Path;

const IMAGE_MARKER_PREFIX: &str = "[IMAGE:";
const ALLOWED_IMAGE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];

#[derive(Debug, Clone)]
pub struct PreparedMessages {
    pub messages: Vec<ChatMessage>,
    pub contains_images: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MultimodalError {
    #[error("multimodal image limit exceeded: max_images={max_images}, found={found}")]
    TooManyImages { max_images: usize, found: usize },

    #[error("multimodal image size limit exceeded for '{input}': {size_bytes} bytes > {max_bytes} bytes")]
    ImageTooLarge {
        input: String,
        size_bytes: usize,
        max_bytes: usize,
    },

    #[error("multimodal image MIME type is not allowed for '{input}': {mime}")]
    UnsupportedMime { input: String, mime: String },

    #[error("multimodal remote image fetch is disabled for '{input}'")]
    RemoteFetchDisabled { input: String },

    #[error("multimodal image source not found or unreadable: '{input}'")]
    ImageSourceNotFound { input: String },

    #[error("invalid multimodal image marker '{input}': {reason}")]
    InvalidMarker { input: String, reason: String },

    #[error("failed to download remote image '{input}': {reason}")]
    RemoteFetchFailed { input: String, reason: String },

    #[error("failed to read local image '{input}': {reason}")]
    LocalReadFailed { input: String, reason: String },
}

pub fn parse_image_markers(content: &str) -> (String, Vec<String>) {
    let mut refs = Vec::new();
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = content[cursor..].find(IMAGE_MARKER_PREFIX) {
        let start = cursor + rel_start;
        cleaned.push_str(&content[cursor..start]);

        let marker_start = start + IMAGE_MARKER_PREFIX.len();
        let Some(rel_end) = content[marker_start..].find(']') else {
            cleaned.push_str(&content[start..]);
            cursor = content.len();
            break;
        };

        let end = marker_start + rel_end;
        let candidate = content[marker_start..end].trim();

        if candidate.is_empty() {
            cleaned.push_str(&content[start..=end]);
        } else {
            refs.push(candidate.to_string());
        }

        cursor = end + 1;
    }

    if cursor < content.len() {
        cleaned.push_str(&content[cursor..]);
    }

    (cleaned.trim().to_string(), refs)
}

pub fn count_image_markers(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| parse_image_markers(&m.content).1.len())
        .sum()
}

pub fn contains_image_markers(messages: &[ChatMessage]) -> bool {
    count_image_markers(messages) > 0
}

pub fn extract_ollama_image_payload(image_ref: &str) -> Option<String> {
    if image_ref.starts_with("data:") {
        let comma_idx = image_ref.find(',')?;
        let (_, payload) = image_ref.split_at(comma_idx + 1);
        let payload = payload.trim();
        if payload.is_empty() {
            None
        } else {
            Some(payload.to_string())
        }
    } else {
        Some(image_ref.trim().to_string()).filter(|value| !value.is_empty())
    }
}

pub async fn prepare_messages_for_provider(
    messages: &[ChatMessage],
    config: &MultimodalConfig,
) -> anyhow::Result<PreparedMessages> {
    let (max_images, max_image_size_mb) = config.effective_limits();
    let max_bytes = max_image_size_mb.saturating_mul(1024 * 1024);

    let found_images = count_image_markers(messages);
    if found_images > max_images {
        return Err(MultimodalError::TooManyImages {
            max_images,
            found: found_images,
        }
        .into());
    }

    if found_images == 0 {
        return Ok(PreparedMessages {
            messages: messages.to_vec(),
            contains_images: false,
        });
    }

    let remote_client = build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10);

    let mut normalized_messages = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role != "user" {
            normalized_messages.push(message.clone());
            continue;
        }

        let (cleaned_text, refs) = parse_image_markers(&message.content);
        if refs.is_empty() {
            normalized_messages.push(message.clone());
            continue;
        }

        let mut normalized_refs = Vec::with_capacity(refs.len());
        for reference in refs {
            let data_uri =
                normalize_image_reference(&reference, config, max_bytes, &remote_client).await?;
            normalized_refs.push(data_uri);
        }

        let content = compose_multimodal_message(&cleaned_text, &normalized_refs);
        normalized_messages.push(ChatMessage {
            role: message.role.clone(),
            content,
        });
    }

    Ok(PreparedMessages {
        messages: normalized_messages,
        contains_images: true,
    })
}

fn compose_multimodal_message(text: &str, data_uris: &[String]) -> String {
    let mut content = String::new();
    let trimmed = text.trim();

    if !trimmed.is_empty() {
        content.push_str(trimmed);
        content.push_str("\n\n");
    }

    for (index, data_uri) in data_uris.iter().enumerate() {
        if index > 0 {
            content.push('\n');
        }
        content.push_str(IMAGE_MARKER_PREFIX);
        content.push_str(data_uri);
        content.push(']');
    }

    content
}

async fn normalize_image_reference(
    source: &str,
    config: &MultimodalConfig,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<String> {
    if source.starts_with("data:") {
        return normalize_data_uri(source, max_bytes);
    }

    if source.starts_with("http://") || source.starts_with("https://") {
        if !config.allow_remote_fetch {
            return Err(MultimodalError::RemoteFetchDisabled {
                input: source.to_string(),
            }
            .into());
        }

        return normalize_remote_image(source, max_bytes, remote_client).await;
    }

    normalize_local_image(source, max_bytes).await
}

fn normalize_data_uri(source: &str, max_bytes: usize) -> anyhow::Result<String> {
    let Some(comma_idx) = source.find(',') else {
        return Err(MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: "expected data URI payload".to_string(),
        }
        .into());
    };

    let header = &source[..comma_idx];
    let payload = source[comma_idx + 1..].trim();

    if !header.contains(";base64") {
        return Err(MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: "only base64 data URIs are supported".to_string(),
        }
        .into());
    }

    let mime = header
        .trim_start_matches("data:")
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    validate_mime(source, &mime)?;

    let decoded = STANDARD
        .decode(payload)
        .map_err(|error| MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: format!("invalid base64 payload: {error}"),
        })?;

    validate_size(source, decoded.len(), max_bytes)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(decoded)))
}

async fn normalize_remote_image(
    source: &str,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<String> {
    let response = remote_client.get(source).send().await.map_err(|error| {
        MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: format!("HTTP {status}"),
        }
        .into());
    }

    if let Some(content_length) = response.content_length() {
        let content_length = content_length as usize;
        validate_size(source, content_length, max_bytes)?;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    let bytes = response
        .bytes()
        .await
        .map_err(|error| MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_size(source, bytes.len(), max_bytes)?;

    let mime = detect_mime(None, bytes.as_ref(), content_type.as_deref()).ok_or_else(|| {
        MultimodalError::UnsupportedMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
        }
    })?;

    validate_mime(source, &mime)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

async fn normalize_local_image(source: &str, max_bytes: usize) -> anyhow::Result<String> {
    let path = Path::new(source);
    if !path.exists() || !path.is_file() {
        return Err(MultimodalError::ImageSourceNotFound {
            input: source.to_string(),
        }
        .into());
    }

    let metadata =
        tokio::fs::metadata(path)
            .await
            .map_err(|error| MultimodalError::LocalReadFailed {
                input: source.to_string(),
                reason: error.to_string(),
            })?;

    validate_size(source, metadata.len() as usize, max_bytes)?;

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| MultimodalError::LocalReadFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_size(source, bytes.len(), max_bytes)?;

    let mime =
        detect_mime(Some(path), &bytes, None).ok_or_else(|| MultimodalError::UnsupportedMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
        })?;

    validate_mime(source, &mime)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn validate_size(source: &str, size_bytes: usize, max_bytes: usize) -> anyhow::Result<()> {
    if size_bytes > max_bytes {
        return Err(MultimodalError::ImageTooLarge {
            input: source.to_string(),
            size_bytes,
            max_bytes,
        }
        .into());
    }

    Ok(())
}

fn validate_mime(source: &str, mime: &str) -> anyhow::Result<()> {
    if ALLOWED_IMAGE_MIME_TYPES.contains(&mime) {
        return Ok(());
    }

    Err(MultimodalError::UnsupportedMime {
        input: source.to_string(),
        mime: mime.to_string(),
    }
    .into())
}

fn detect_mime(
    path: Option<&Path>,
    bytes: &[u8],
    header_content_type: Option<&str>,
) -> Option<String> {
    if let Some(header_mime) = header_content_type.and_then(normalize_content_type) {
        return Some(header_mime);
    }

    if let Some(path) = path {
        if let Some(ext) = path.extension().and_then(|value| value.to_str()) {
            if let Some(mime) = mime_from_extension(ext) {
                return Some(mime.to_string());
            }
        }
    }

    mime_from_magic(bytes).map(ToString::to_string)
}

fn normalize_content_type(content_type: &str) -> Option<String> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    if mime.is_empty() {
        None
    } else {
        Some(mime)
    }
}

fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn mime_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some("image/png");
    }

    if bytes.len() >= 3 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }

    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some("image/gif");
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }

    if bytes.len() >= 2 && bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }

    None
}

#[cfg(test)]
#[path = "multimodal_tests.rs"]
mod tests;
