use crate::openhuman::config::{
    build_runtime_proxy_client_with_timeouts, MultimodalConfig, MultimodalFileConfig,
};
use crate::openhuman::inference::provider::ChatMessage;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

const IMAGE_MARKER_PREFIX: &str = "[IMAGE:";
const ALLOWED_IMAGE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];

/// File-attachment marker prefix. Counterpart to [`IMAGE_MARKER_PREFIX`].
/// Resolution rules mirror images: local paths, optional http(s) URLs
/// gated by [`MultimodalFileConfig::allow_remote_fetch`]. `data:` URIs
/// are intentionally rejected — the file pipeline does not inline
/// base64 the way images do; users wanting inline content should paste
/// it as text.
const FILE_MARKER_PREFIX: &str = "[FILE:";

/// Hard upper bound on how long [`pdf_extract::extract_text_from_mem`]
/// may run before the worker is abandoned and the file degrades to a
/// metadata-only reference. PDFs known to choke the parser (extremely
/// large, encrypted, malformed) must not stall a chat turn.
const PDF_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Worst-case length budget reserved for the rendered truncation
/// suffix. The actual emitted suffix is `"\n[…truncated {N} chars]"`
/// where `N` is the dynamic dropped-character count. The reservation
/// uses the longest plausible value (`max_extracted_text_chars` is
/// clamped to 200_000, so `N` has up to 6 digits) so the truncated
/// payload never overshoots `max_extracted_text_chars` even after the
/// suffix is appended.
const TEXT_TRUNCATION_SUFFIX_BUDGET: &str = "\n[…truncated 999999 chars]";

#[derive(Debug, Clone)]
pub struct PreparedMessages {
    pub messages: Vec<ChatMessage>,
    pub contains_images: bool,
    pub contains_files: bool,
}

/// Resolved representation of a `[FILE:…]` marker. Extractable formats
/// inline their text payload; binary-only formats surface as metadata
/// only so the agent can mention them without seeing raw bytes.
#[derive(Debug, Clone)]
pub enum FilePayload {
    Extracted {
        name: String,
        mime: String,
        size_bytes: usize,
        text: String,
        truncated_chars: usize,
    },
    Reference {
        name: String,
        mime: String,
        size_bytes: usize,
        sha256_prefix: String,
    },
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

    #[error("multimodal file limit exceeded: max_files={max_files}, found={found}")]
    TooManyFiles { max_files: usize, found: usize },

    #[error(
        "multimodal file size limit exceeded for '{input}': {size_bytes} bytes > {max_bytes} bytes"
    )]
    FileTooLarge {
        input: String,
        size_bytes: usize,
        max_bytes: usize,
    },

    #[error(
        "multimodal file MIME type '{mime}' for '{input}' is not allowed; supported: {supported}"
    )]
    UnsupportedFileMime {
        input: String,
        mime: String,
        supported: String,
    },

    #[error("multimodal file source not found or unreadable: '{input}'")]
    FileSourceNotFound { input: String },

    #[error("multimodal remote file fetch is disabled for '{input}'")]
    RemoteFileFetchDisabled { input: String },

    #[error("failed to download remote file '{input}': {reason}")]
    RemoteFileFetchFailed { input: String, reason: String },

    #[error("failed to read local file '{input}': {reason}")]
    LocalFileReadFailed { input: String, reason: String },

    #[error("invalid multimodal file marker '{input}': {reason}")]
    InvalidFileMarker { input: String, reason: String },
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

/// Count `[IMAGE:…]` markers in the **latest** user message only.
///
/// Earlier versions summed markers across every user-role message in
/// the history, which made the per-turn `max_images` cap drift upward
/// over a long conversation: a thread that attached three images on
/// turn 1 already counted them again on turn 2 even when the new user
/// message had no attachments at all. Looking only at the most recent
/// user message matches the user's intent ("how many am I attaching
/// THIS turn") and keeps the cap stable.
pub fn count_image_markers(messages: &[ChatMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| parse_image_markers(&m.content).1.len())
        .unwrap_or(0)
}

pub fn contains_image_markers(messages: &[ChatMessage]) -> bool {
    count_image_markers(messages) > 0
}

fn latest_user_message(messages: &[ChatMessage]) -> Option<&ChatMessage> {
    messages.iter().rev().find(|m| m.role == "user")
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

/// Strip every `[FILE:…]` marker from `content` and return the cleaned
/// text alongside the raw source references in order. Mirrors
/// [`parse_image_markers`] so the two pipelines stay symmetrical.
pub fn parse_file_markers(content: &str) -> (String, Vec<String>) {
    let mut refs = Vec::new();
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = content[cursor..].find(FILE_MARKER_PREFIX) {
        let start = cursor + rel_start;
        cleaned.push_str(&content[cursor..start]);

        let marker_start = start + FILE_MARKER_PREFIX.len();
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

/// Count `[FILE:…]` markers in the **latest** user message only — same
/// per-turn semantics as [`count_image_markers`]. See that function's
/// rustdoc for the reasoning.
pub fn count_file_markers(messages: &[ChatMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| parse_file_markers(&m.content).1.len())
        .unwrap_or(0)
}

pub fn contains_file_markers(messages: &[ChatMessage]) -> bool {
    count_file_markers(messages) > 0
}

pub async fn prepare_messages_for_provider(
    messages: &[ChatMessage],
    image_config: &MultimodalConfig,
    file_config: &MultimodalFileConfig,
) -> anyhow::Result<PreparedMessages> {
    let (max_images, max_image_size_mb) = image_config.effective_limits();
    let max_image_bytes = max_image_size_mb.saturating_mul(1024 * 1024);

    let (max_files, max_file_size_mb, max_extracted_text_chars) = file_config.effective_limits();
    let max_file_bytes = max_file_size_mb.saturating_mul(1024 * 1024);

    let found_images = count_image_markers(messages);
    if found_images > max_images {
        return Err(MultimodalError::TooManyImages {
            max_images,
            found: found_images,
        }
        .into());
    }

    let found_files = count_file_markers(messages);
    // Hard-zero gate: `MultimodalFileConfig::for_untrusted_channel_input()`
    // (and the triage arm) sets `max_files: 0` as a sentinel meaning
    // "reject every `[FILE:…]` marker before any disk read." The clamp
    // inside `effective_limits` lifts 0 → 1, so without this pre-check a
    // single attacker-supplied `[FILE:/etc/passwd]` would slip through
    // (`1 > 1` is false). Honour the raw value here so the channel /
    // triage hardening is actually enforced.
    if file_config.max_files == 0 && found_files > 0 {
        return Err(MultimodalError::TooManyFiles {
            max_files: 0,
            found: found_files,
        }
        .into());
    }
    if found_files > max_files {
        return Err(MultimodalError::TooManyFiles {
            max_files,
            found: found_files,
        }
        .into());
    }

    tracing::debug!(
        target: "multimodal",
        found_images,
        found_files,
        "[multimodal] preparing messages"
    );

    if found_images == 0 && found_files == 0 {
        return Ok(PreparedMessages {
            messages: messages.to_vec(),
            contains_images: false,
            contains_files: false,
        });
    }

    let remote_client = build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10);

    let mut normalized_messages = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role != "user" {
            normalized_messages.push(message.clone());
            continue;
        }

        let (text_after_images, image_refs) = parse_image_markers(&message.content);
        let (cleaned_text, file_refs) = parse_file_markers(&text_after_images);

        if image_refs.is_empty() && file_refs.is_empty() {
            normalized_messages.push(message.clone());
            continue;
        }

        let mut normalized_image_refs = Vec::with_capacity(image_refs.len());
        for reference in image_refs {
            let data_uri = normalize_image_reference(
                &reference,
                image_config,
                max_image_bytes,
                &remote_client,
            )
            .await?;
            normalized_image_refs.push(data_uri);
        }

        let mut file_payloads = Vec::with_capacity(file_refs.len());
        for reference in file_refs {
            let payload = normalize_file_reference(
                &reference,
                file_config,
                max_file_bytes,
                max_extracted_text_chars,
                &remote_client,
            )
            .await?;
            file_payloads.push(payload);
        }

        let content =
            compose_multimodal_message(&cleaned_text, &normalized_image_refs, &file_payloads);
        normalized_messages.push(ChatMessage {
            id: message.id.clone(),
            role: message.role.clone(),
            content,
            extra_metadata: message.extra_metadata.clone(),
        });
    }

    Ok(PreparedMessages {
        messages: normalized_messages,
        contains_images: found_images > 0,
        contains_files: found_files > 0,
    })
}

fn compose_multimodal_message(
    text: &str,
    data_uris: &[String],
    file_payloads: &[FilePayload],
) -> String {
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

    for payload in file_payloads {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        if !content.is_empty() {
            content.push('\n');
        }
        match payload {
            FilePayload::Extracted {
                name,
                mime,
                size_bytes,
                text,
                truncated_chars,
            } => {
                content.push_str(&format!(
                    "[FILE-EXTRACTED: name=\"{}\" size=\"{}\" mime=\"{}\"]\n",
                    escape_attr(name),
                    format_size(*size_bytes),
                    mime
                ));
                content.push_str(text);
                if *truncated_chars > 0 {
                    content.push_str(&format!("\n[…truncated {} chars]", truncated_chars));
                }
                content.push_str("\n[/FILE-EXTRACTED]");
            }
            FilePayload::Reference {
                name,
                mime,
                size_bytes,
                sha256_prefix,
            } => {
                content.push_str(&format!(
                    "[FILE-ATTACHED: name=\"{}\" size=\"{}\" mime=\"{}\" sha256_prefix=\"{}\"]",
                    escape_attr(name),
                    format_size(*size_bytes),
                    mime,
                    sha256_prefix
                ));
            }
        }
    }

    content
}

/// Strip characters that would break the attribute-style serialization
/// of a [`FilePayload`] header (`"` and newlines). Names are user-
/// supplied filenames so they must not be trusted to be quote-free.
fn escape_attr(value: &str) -> String {
    value.replace(['"', '\n', '\r'], "_")
}

fn format_size(size_bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = 1024 * 1024;
    if size_bytes >= MB {
        format!("{:.1} MB", size_bytes as f64 / MB as f64)
    } else if size_bytes >= KB {
        format!("{:.1} KB", size_bytes as f64 / KB as f64)
    } else {
        format!("{} B", size_bytes)
    }
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

// ── File-attachment pipeline ──────────────────────────────────────────
//
// File markers run through a parallel pipeline to image markers but
// with a different end-state: extractable formats inline their text
// payload, binary-only formats surface as metadata-only references.
// The agent never sees raw binary bytes for `[FILE:…]` markers — base64
// inlining is the image pipeline's exclusive contract.

async fn normalize_file_reference(
    source: &str,
    config: &MultimodalFileConfig,
    max_bytes: usize,
    max_extracted_text_chars: usize,
    remote_client: &Client,
) -> anyhow::Result<FilePayload> {
    if source.starts_with("data:") {
        return Err(MultimodalError::InvalidFileMarker {
            input: source.to_string(),
            reason: "data: URIs are not supported for [FILE:…] markers — paste content as text"
                .to_string(),
        }
        .into());
    }

    let (bytes, path_hint, name, header_content_type) =
        if source.starts_with("http://") || source.starts_with("https://") {
            if !config.allow_remote_fetch {
                return Err(MultimodalError::RemoteFileFetchDisabled {
                    input: source.to_string(),
                }
                .into());
            }
            let (bytes, name, content_type) =
                fetch_remote_file(source, max_bytes, remote_client).await?;
            (bytes, None, name, content_type)
        } else {
            let (bytes, path, name) = read_local_file(source, max_bytes).await?;
            (bytes, Some(path), name, None)
        };

    let mime = detect_file_mime(path_hint.as_deref(), &bytes, header_content_type.as_deref())
        .ok_or_else(|| MultimodalError::UnsupportedFileMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
            supported: config.allowed_mime_types.join(", "),
        })?;

    if !config.is_mime_allowed(&mime) {
        return Err(MultimodalError::UnsupportedFileMime {
            input: source.to_string(),
            mime: mime.clone(),
            supported: config.allowed_mime_types.join(", "),
        }
        .into());
    }

    let size_bytes = bytes.len();

    tracing::debug!(
        target: "multimodal",
        file = %name,
        mime = %mime,
        size_bytes,
        "[multimodal::files] resolved file ref"
    );

    if is_extractable_text_mime(&mime) {
        match extract_utf8_text(&bytes) {
            Ok(raw) => {
                let (text, truncated_chars) = truncate_chars(raw, max_extracted_text_chars);
                if truncated_chars > 0 {
                    tracing::info!(
                        target: "multimodal",
                        file = %name,
                        truncated_chars,
                        max_extracted_text_chars,
                        "[multimodal::files] truncated extracted text"
                    );
                }
                return Ok(FilePayload::Extracted {
                    name,
                    mime,
                    size_bytes,
                    text,
                    truncated_chars,
                });
            }
            Err(reason) => {
                tracing::warn!(
                    target: "multimodal",
                    file = %name,
                    reason = %reason,
                    "[multimodal::files] utf-8 decode failed, degrading to reference"
                );
            }
        }
    }

    if mime == "application/pdf" {
        match extract_pdf_text(bytes.clone()).await {
            Ok(raw) => {
                let (text, truncated_chars) = truncate_chars(raw, max_extracted_text_chars);
                if truncated_chars > 0 {
                    tracing::info!(
                        target: "multimodal",
                        file = %name,
                        truncated_chars,
                        max_extracted_text_chars,
                        "[multimodal::files] truncated extracted text"
                    );
                }
                return Ok(FilePayload::Extracted {
                    name,
                    mime,
                    size_bytes,
                    text,
                    truncated_chars,
                });
            }
            Err(reason) => {
                tracing::warn!(
                    target: "multimodal",
                    file = %name,
                    reason = %reason,
                    "[multimodal::files] PDF extraction failed, degrading to reference"
                );
            }
        }
    }

    let sha256_prefix = sha256_prefix(&bytes);
    Ok(FilePayload::Reference {
        name,
        mime,
        size_bytes,
        sha256_prefix,
    })
}

async fn read_local_file(
    source: &str,
    max_bytes: usize,
) -> anyhow::Result<(Vec<u8>, std::path::PathBuf, String)> {
    let path = Path::new(source).to_path_buf();
    if !path.exists() || !path.is_file() {
        return Err(MultimodalError::FileSourceNotFound {
            input: source.to_string(),
        }
        .into());
    }

    let metadata =
        tokio::fs::metadata(&path)
            .await
            .map_err(|error| MultimodalError::LocalFileReadFailed {
                input: source.to_string(),
                reason: error.to_string(),
            })?;

    validate_file_size(source, metadata.len() as usize, max_bytes)?;

    let bytes =
        tokio::fs::read(&path)
            .await
            .map_err(|error| MultimodalError::LocalFileReadFailed {
                input: source.to_string(),
                reason: error.to_string(),
            })?;

    validate_file_size(source, bytes.len(), max_bytes)?;

    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| source.to_string());

    Ok((bytes, path, name))
}

async fn fetch_remote_file(
    source: &str,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<(Vec<u8>, String, Option<String>)> {
    let response = remote_client.get(source).send().await.map_err(|error| {
        MultimodalError::RemoteFileFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(MultimodalError::RemoteFileFetchFailed {
            input: source.to_string(),
            reason: format!("HTTP {status}"),
        }
        .into());
    }

    if let Some(content_length) = response.content_length() {
        let content_length = content_length as usize;
        validate_file_size(source, content_length, max_bytes)?;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    let bytes = response
        .bytes()
        .await
        .map_err(|error| MultimodalError::RemoteFileFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_file_size(source, bytes.len(), max_bytes)?;

    let name = source
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(source)
        .to_string();

    Ok((bytes.to_vec(), name, content_type))
}

fn validate_file_size(source: &str, size_bytes: usize, max_bytes: usize) -> anyhow::Result<()> {
    if size_bytes > max_bytes {
        return Err(MultimodalError::FileTooLarge {
            input: source.to_string(),
            size_bytes,
            max_bytes,
        }
        .into());
    }

    Ok(())
}

fn is_extractable_text_mime(mime: &str) -> bool {
    matches!(mime, "text/plain" | "text/csv" | "text/markdown")
}

/// Best-effort UTF-8 decode. Strict decode wins; on failure falls back
/// to `from_utf8_lossy` (replaces invalid sequences with U+FFFD). The
/// returned `Err` is reserved for future hard-fail modes — currently
/// the function never returns `Err`, but keeping the result type
/// preserves the option to surface lossy decoding to the caller.
fn extract_utf8_text(bytes: &[u8]) -> Result<String, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) => Ok(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// Run `pdf-extract` on a copy of `bytes` inside a `spawn_blocking`
/// worker, bounded by [`PDF_EXTRACTION_TIMEOUT`]. Returns the raw
/// extracted text on success; on timeout / panic / parse error the
/// caller degrades the file to [`FilePayload::Reference`] rather than
/// surface the failure to the user (avoids Sentry noise on broken PDFs).
async fn extract_pdf_text(bytes: Vec<u8>) -> Result<String, String> {
    let extraction = tokio::task::spawn_blocking(move || {
        pdf_extract::extract_text_from_mem(&bytes).map_err(|error| error.to_string())
    });

    match tokio::time::timeout(PDF_EXTRACTION_TIMEOUT, extraction).await {
        Ok(Ok(Ok(text))) => Ok(text),
        Ok(Ok(Err(reason))) => Err(reason),
        Ok(Err(join_error)) => Err(format!("pdf extraction worker panicked: {join_error}")),
        Err(_) => Err(format!(
            "pdf extraction exceeded {}s timeout",
            PDF_EXTRACTION_TIMEOUT.as_secs()
        )),
    }
}

/// Truncate `text` to at most `max_chars` Unicode scalar values, leaving
/// room for the rendered `"\n[…truncated {dropped} chars]"` suffix.
/// The reservation uses [`TEXT_TRUNCATION_SUFFIX_BUDGET`] — the
/// worst-case rendered length — so the final `text + suffix` payload
/// always stays inside `max_chars` regardless of the actual dropped
/// digit count. Returns the (possibly-trimmed) text and the count of
/// chars dropped (0 when no truncation happened).
fn truncate_chars(text: String, max_chars: usize) -> (String, usize) {
    let total = text.chars().count();
    if total <= max_chars {
        return (text, 0);
    }

    let suffix_chars = TEXT_TRUNCATION_SUFFIX_BUDGET.chars().count();
    let keep = max_chars.saturating_sub(suffix_chars);
    let truncated: String = text.chars().take(keep).collect();
    let dropped = total.saturating_sub(keep);
    (truncated, dropped)
}

fn sha256_prefix(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{:02x}", byte)).collect();
    hex.chars().take(16).collect()
}

fn detect_file_mime(
    path: Option<&Path>,
    bytes: &[u8],
    header_content_type: Option<&str>,
) -> Option<String> {
    if let Some(header_mime) = header_content_type.and_then(normalize_content_type) {
        if file_mime_known(&header_mime) {
            return Some(header_mime);
        }
    }

    if let Some(path) = path {
        if let Some(ext) = path.extension().and_then(|value| value.to_str()) {
            if let Some(mime) = file_mime_from_extension(ext) {
                return Some(mime.to_string());
            }
        }
    }

    if let Some(mime) = file_mime_from_magic(bytes) {
        return Some(mime.to_string());
    }

    if looks_like_utf8_text(bytes) {
        return Some("text/plain".to_string());
    }

    None
}

fn file_mime_known(mime: &str) -> bool {
    file_mime_from_extension(mime).is_some()
        || matches!(
            mime,
            "application/pdf"
                | "text/plain"
                | "text/csv"
                | "text/markdown"
                | "application/zip"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/octet-stream"
        )
}

fn file_mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        "md" | "markdown" => Some("text/markdown"),
        "csv" => Some("text/csv"),
        "zip" => Some("application/zip"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        _ => None,
    }
}

fn file_mime_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 5 && bytes.starts_with(b"%PDF-") {
        return Some("application/pdf");
    }

    // OOXML formats (xlsx/docx/pptx) and plain zip all share the
    // PK\x03\x04 ZIP local-file-header magic; without parsing the
    // central directory we cannot distinguish them, so callers must
    // rely on the file extension for OOXML vs zip discrimination.
    if bytes.len() >= 4 && bytes.starts_with(&[b'P', b'K', 0x03, 0x04]) {
        return Some("application/zip");
    }

    None
}

/// Crude UTF-8 sniff: bytes parse as UTF-8 and contain at least one
/// printable character. Used as a last-resort fallback so unlabeled
/// .log / .ini / source files are still recognised as text.
fn looks_like_utf8_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text
            .chars()
            .any(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "multimodal_tests.rs"]
mod tests;
